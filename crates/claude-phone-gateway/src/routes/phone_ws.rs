use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{
    ControlMessage, ErrorCode, ErrorMessage, PeerStatus, PhoneHello, ServerHello,
};
use claude_phone_shared::SessionToken;

use crate::rate_limit::{
    ConnRateLimiter, PHONE_TO_GW_MSG_PER_SEC, PONG_DEADLINE, SINK_SEND_TIMEOUT,
};
use crate::session::{short_id, Frame, SessionRegistry};

#[derive(Clone)]
pub struct PhoneWsState {
    pub registry: Arc<SessionRegistry>,
    /// When `Some`, browser-initiated WSes whose `Origin` header doesn't
    /// equal this value are rejected with 403. When `None`, no enforcement.
    pub public_origin: Option<String>,
}

/// Hard cap on a single WebSocket message. PTY stdout chunks are 8KB; phone
/// keystrokes are tiny. 64KB is way above what either side needs and well
/// below what an attacker could use to OOM the gateway.
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;

/// Time the phone has to send its `phone_hello` after the WS upgrade
/// resolves and the session attach succeeds. Mirrors `HELLO_TIMEOUT` in
/// wrapper_ws — a real browser sends the hello in milliseconds. Without
/// this bound an attacker holding a leaked token could open the socket
/// and squat without identifying itself, costing a registry slot.
const PHONE_HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn handler(
    ws: WebSocketUpgrade,
    Path(token_str): Path<String>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<PhoneWsState>,
) -> Response {
    // TM-AUTH.7 — per-connection-attempt correlation ID. See wrapper_ws for
    // the full rationale. The conn_id rides every auth-failure log line on
    // this attempt so an operator can grep one ID across log destinations.
    let conn_id = short_id();

    // TM-WS.11: strict 43-char length check rejects malformed token shapes
    // before allocating the WebSocket upgrade. The previous 8..=64 band let
    // off-shape strings reach SessionToken::parse() unnecessarily.
    // TM-INPUT.7: charset is enforced one level down by `SessionToken::parse`
    // via `is_base64url_byte` (alphanumeric + `-` + `_`). Anything else —
    // NUL, BEL, ESC, DEL, slash, backslash, whitespace, high-bit bytes — is
    // rejected and never reaches the WebSocket attach.
    if token_str.len() != SessionToken::LEN {
        // TM-AUTH.7 — `token_str` is NEVER threaded into the log. Even a
        // malformed candidate could be a near-miss of a real token; a
        // shoulder-surfer who glimpsed the first few chars would otherwise
        // get exactly the confirmation they need from our log.
        tracing::warn!(
            event = "auth_failure",
            conn_id = %conn_id,
            peer_ip = %peer.ip(),
            reason = "bad_token_format",
            route = "phone_ws",
            "TM-AUTH.7 auth failure"
        );
        return StatusCode::BAD_REQUEST.into_response();
    }

    // TM-WS.1, .2 — Origin equality check when public_origin is configured.
    // TM-WS.3 — fail-closed on MISSING Origin: phone_ws is browser-served
    // (the page at /s/:token opens a same-origin WebSocket), so a legitimate
    // browser always sends Origin. A missing Origin is either a non-browser
    // client probing the endpoint with a stolen token, or a browser quirk
    // we don't support. Both deserve 403. Wrapper_ws deliberately stays
    // permissive — wrappers are CLI processes that don't send Origin.
    if let Some(expected) = state.public_origin.as_deref() {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        match origin {
            Some(o) if o == expected => {}
            _ => {
                // TM-AUTH.7
                tracing::warn!(
                    event = "auth_failure",
                    conn_id = %conn_id,
                    peer_ip = %peer.ip(),
                    reason = "forbidden_origin",
                    route = "phone_ws",
                    "TM-AUTH.7 auth failure"
                );
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state, token_str, peer, conn_id))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: PhoneWsState,
    token_str: String,
    peer: SocketAddr,
    conn_id: String,
) {
    let token = match SessionToken::parse(&token_str) {
        Ok(t) => t,
        Err(_) => {
            // TM-AUTH.7 — defense-in-depth: the 43-char length gate in the
            // handler already rejects most malformed shapes, but a 43-char
            // string with a charset violation reaches here. `token_str` is
            // NOT logged: it's the rejected secret candidate.
            tracing::warn!(
                event = "auth_failure",
                conn_id = %conn_id,
                peer_ip = %peer.ip(),
                reason = "bad_token_format",
                route = "phone_ws",
                "TM-AUTH.7 auth failure"
            );
            send_error(
                &mut socket,
                ErrorCode::InvalidToken,
                "bad token format".into(),
            )
            .await;
            return;
        }
    };

    let handle = match state.registry.attach_phone(&token).await {
        Ok(h) => h,
        Err(_) => {
            // TM-AUTH.7 — the well-formed token does not map to any
            // registered session. Reason is intentionally coarse: callers
            // cannot tell from this whether the session was never created,
            // expired, or was just torn down — that asymmetry is the point.
            tracing::warn!(
                event = "auth_failure",
                conn_id = %conn_id,
                peer_ip = %peer.ip(),
                reason = "session_not_found",
                route = "phone_ws",
                "TM-AUTH.7 auth failure"
            );
            send_error(
                &mut socket,
                ErrorCode::InvalidToken,
                "no such session".into(),
            )
            .await;
            return;
        }
    };

    let session_id = handle.session.id.clone();

    // Require `phone_hello` as the first frame after attach. The hello:
    // (a) commits the phone to the protocol — a random probe that just
    //     opened the socket gets bounced as ProtocolViolation instead of
    //     receiving ServerHello and being able to forward keystrokes;
    // (b) verifies the token in the hello body matches the URL token,
    //     defense-in-depth against future protocol changes that might
    //     decouple them, and a small barrier to scripted abusers.
    // We do NOT send `peer_up` to the wrapper until after the hello
    // succeeds, so a bouncing-bad-phone never produces a phantom
    // peer-connect/peer-disconnect pair in the wrapper's log.
    if let Err(why) = recv_phone_hello(&mut socket, &token).await {
        // TM-AUTH.7 — canonical auth-failure shape. The `reason` here is one
        // of the &'static str values returned by `recv_phone_hello`: each is
        // a stable taxonomy token (no token bytes, no api_key bytes). The
        // `session_id` field is the gateway's own id (not a secret), kept
        // for cross-correlation with the wrapper-side "wrapper attached"
        // log emitted when the same session was registered.
        tracing::warn!(
            event = "auth_failure",
            conn_id = %conn_id,
            session_id = %session_id,
            peer_ip = %peer.ip(),
            reason = why,
            route = "phone_ws",
            "TM-AUTH.7 auth failure (phone_hello)"
        );
        send_error(&mut socket, ErrorCode::ProtocolViolation, why.into()).await;
        // Cleanup attach without notifying wrapper. We never sent peer_up,
        // so we owe no peer_down.
        let mut slot = handle.session.to_phone.lock().await;
        slot.detach();
        drop(slot);
        handle.session.touch_phone().await;
        return;
    }

    let server_hello = ControlMessage::ServerHello(ServerHello {
        session_id: session_id.clone(),
        peer_connected: true,
    });
    // TM-CODE.3: ServerHello is a derive(Serialize) struct — infallible.
    if socket
        .send(Message::Text(
            serde_json::to_string(&server_hello).expect("ServerHello serializes (static struct)"),
        ))
        .await
        .is_err()
    {
        return;
    }

    let peer_up = ControlMessage::PeerStatus(PeerStatus { connected: true });
    // TM-CODE.3: PeerStatus is a derive(Serialize) struct with a single bool.
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(
            serde_json::to_string(&peer_up).expect("PeerStatus serializes (static struct)"),
        ))
        .await;

    tracing::info!(session_id = %session_id, "phone attached");

    let (mut sink, mut stream) = socket.split();
    let mut rx_from_wrapper = handle.rx;
    let to_wrapper = handle.session.to_wrapper.clone();
    let cancel = handle.session.cancel.clone();

    // TM-RATE.7 — shared "millis since this socket opened" reference for the
    // no-pong watchdog. See wrapper_ws for the full explanation; the same
    // shape is needed here so a phone with a dead reverse-channel doesn't
    // hold an FD indefinitely.
    let socket_start = Instant::now();
    let last_pong_ms = Arc::new(AtomicU64::new(0));
    let last_pong_outgoing = last_pong_ms.clone();

    let cancel_outgoing = cancel.clone();
    let cancel_for_flood = cancel.clone();
    let outgoing_task = tokio::spawn(async move {
        // TM-RATE.3 — per-connection sliding-window cap on phone→wrapper
        // traffic. Direction is keystrokes/touches, so PHONE_TO_GW_MSG_PER_SEC
        // (100/s) is generous for a human while still ruling out automated
        // floods. Exceeding the cap cancels the whole session, not just this
        // task — a flooding phone has no business holding the session open.
        let mut conn_rate = ConnRateLimiter::new(PHONE_TO_GW_MSG_PER_SEC);
        loop {
            let cancelled = cancel_outgoing.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                msg = stream.next() => {
                    // TM-RATE.7: producer-side break must propagate cancel
                    // so the whole session tears down promptly when the
                    // phone is gone. Otherwise the wrapper-bound writer
                    // and registry slot wait until the keepalive watchdog
                    // notices the dead socket (up to 30 s + ping timeout),
                    // which widens the slot-exhaustion attack window: a
                    // peer that opens then drops sessions in a tight loop
                    // would otherwise hold `max_sessions` slots open well
                    // past the moment they were abandoned.
                    let Some(Ok(msg)) = msg else {
                        cancel_outgoing.cancel();
                        break;
                    };
                    let frame = match msg {
                        Message::Binary(b) => {
                            // Phone is a remote keyboard, not a terminal-
                            // control channel. Strip OSC / DCS / APC / PM
                            // / SOS sequences (OSC 52 clipboard-set, sixel
                            // graphics, etc.) so a bearer with a leaked
                            // token cannot hijack the host terminal's
                            // clipboard or paint a fake prompt. CSI and
                            // SS3 (arrow keys, function keys, bracketed
                            // paste) are preserved.
                            let cleaned = sanitize_phone_input(&b);
                            if cleaned.is_empty() {
                                continue;
                            }
                            Frame::Binary(cleaned)
                        }
                        Message::Text(t) => Frame::Text(t),
                        Message::Close(_) => {
                            cancel_outgoing.cancel();
                            break;
                        }
                        // TM-RATE.7: stamp last_pong on every received Pong.
                        Message::Pong(_) => {
                            last_pong_outgoing.store(
                                socket_start.elapsed().as_millis() as u64,
                                Ordering::Relaxed,
                            );
                            continue;
                        }
                        _ => continue,
                    };
                    if !conn_rate.check(Instant::now()) {
                        tracing::warn!(
                            peer = %peer,
                            cap = PHONE_TO_GW_MSG_PER_SEC,
                            "TM-RATE.3: phone exceeded per-connection msg rate, closing"
                        );
                        cancel_for_flood.cancel();
                        break;
                    }
                    if to_wrapper.send(frame).await.is_err() { break; }
                }
            }
        }
    });

    let cancel_incoming = cancel.clone();
    let last_pong_incoming = last_pong_ms.clone();
    let incoming_task = tokio::spawn(async move {
        // 30s server-initiated Ping keeps the phone's WebSocket alive across
        // NAT and Cloudflare's idle-connection drop (~100s). The reply Pong
        // (handled in outgoing_task) stamps `last_pong_incoming`; this task
        // checks the age on every tick (TM-RATE.7).
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(30));
        keepalive.tick().await; // skip immediate tick

        loop {
            let cancelled = cancel_incoming.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                frame = rx_from_wrapper.recv() => {
                    let Some(frame) = frame else { break };
                    let msg = match frame {
                        Frame::Binary(b) => Message::Binary(b),
                        Frame::Text(t) => Message::Text(t),
                    };
                    // TM-RATE.6: same slow-write defense as wrapper_ws.
                    // A phone holding its read buffer indefinitely would
                    // otherwise pin the writer forever once the bounded
                    // channel filled up. On stall we cancel the session
                    // so outgoing_task tears down too.
                    match tokio::time::timeout(SINK_SEND_TIMEOUT, sink.send(msg)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            cancel_incoming.cancel();
                            break;
                        }
                        Err(_) => {
                            tracing::warn!(
                                peer = %peer,
                                timeout_secs = SINK_SEND_TIMEOUT.as_secs(),
                                "TM-RATE.6: phone sink send stalled, closing"
                            );
                            cancel_incoming.cancel();
                            break;
                        }
                    }
                }
                _ = keepalive.tick() => {
                    // TM-RATE.7: check elapsed since last Pong BEFORE sending
                    // the next Ping. See wrapper_ws for the full reasoning.
                    let now_ms = socket_start.elapsed().as_millis() as u64;
                    let last_ms = last_pong_incoming.load(Ordering::Relaxed);
                    let age_ms = now_ms.saturating_sub(last_ms);
                    if age_ms > PONG_DEADLINE.as_millis() as u64 {
                        tracing::warn!(
                            peer = %peer,
                            age_ms,
                            deadline_ms = PONG_DEADLINE.as_millis() as u64,
                            "TM-RATE.7: phone no-pong deadline exceeded, closing"
                        );
                        cancel_incoming.cancel();
                        break;
                    }
                    // TM-RATE.6: bounded keepalive write — see wrapper_ws.
                    match tokio::time::timeout(
                        SINK_SEND_TIMEOUT,
                        sink.send(Message::Ping(Vec::new())),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => {
                            cancel_incoming.cancel();
                            break;
                        }
                    }
                }
            }
        }
    });

    let _ = tokio::join!(outgoing_task, incoming_task);

    {
        let mut slot = handle.session.to_phone.lock().await;
        slot.detach();
    }
    // Reset the idle clock — sweeper measures elapsed time from THIS instant
    // for sessions that are currently phone-less.
    handle.session.touch_phone().await;
    let peer_down = ControlMessage::PeerStatus(PeerStatus { connected: false });
    // TM-CODE.3: PeerStatus is a derive(Serialize) struct with a single bool.
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(
            serde_json::to_string(&peer_down).expect("PeerStatus serializes (static struct)"),
        ))
        .await;
    tracing::info!(session_id = %session_id, "phone detached");
}

async fn send_error(socket: &mut WebSocket, code: ErrorCode, message: String) {
    let err = ControlMessage::Error(ErrorMessage { code, message });
    // TM-CODE.3: ErrorMessage is a derive(Serialize) struct — infallible.
    let _ = socket
        .send(Message::Text(
            serde_json::to_string(&err).expect("ErrorMessage serializes (static struct)"),
        ))
        .await;
}

/// Read and validate the phone's `phone_hello`. On success returns `Ok(())`
/// (the body is currently unused beyond presence + token-equality, but the
/// signature is shaped so callers can later thread `cols`/`rows` into a
/// Resize forwarded to the wrapper). On any failure returns `Err(&'static
/// str)` with a stable reason string — fed to both tracing and the
/// ProtocolViolation message body. Reasons are deliberately coarse so they
/// don't act as a probe oracle ("you sent text but wrong shape" vs "wrong
/// token" leaks structural info).
async fn recv_phone_hello(
    socket: &mut WebSocket,
    url_token: &SessionToken,
) -> Result<(), &'static str> {
    let msg = tokio::time::timeout(PHONE_HELLO_TIMEOUT, socket.recv())
        .await
        .map_err(|_| "phone_hello timeout")?
        .ok_or("socket closed before phone_hello")?
        .map_err(|_| "socket error before phone_hello")?;
    let Message::Text(t) = msg else {
        return Err("expected text phone_hello");
    };
    let parsed: ControlMessage =
        serde_json::from_str(&t).map_err(|_| "phone_hello not valid JSON")?;
    let ControlMessage::PhoneHello(PhoneHello { token, .. }) = parsed else {
        return Err("expected phone_hello");
    };
    // Constant-time compare via SessionToken::ct_eq. The URL-side token has
    // already been authoritatively validated above; this check exists so a
    // future protocol change that routed a separate token through the hello
    // body can't quietly desync from the URL.
    if !token.ct_eq(url_token) {
        return Err("phone_hello token mismatch");
    }
    Ok(())
}

/// Strip Operating-System-Command (OSC, `ESC ]`), Device-Control-String
/// (DCS, `ESC P`), Application-Program-Command (APC, `ESC _`),
/// Privacy-Message (PM, `ESC ^`), and Start-of-String (SOS, `ESC X`)
/// sequences from phone-supplied input before it reaches the wrapper PTY.
///
/// The phone is a remote keyboard: there is no legitimate reason for a
/// browser to inject OSC 52 (clipboard set), sixel/DCS graphics, or APC
/// metadata. A bearer with a leaked SessionToken can otherwise hijack the
/// host's clipboard or repaint the screen with a fake prompt and trick
/// the host user into pasting a malicious command.
///
/// CSI (`ESC [ ...`) and SS3 (`ESC O ...`) are intentionally preserved —
/// arrow keys, function keys, Home/End/PgUp/PgDn, and bracketed-paste
/// markers belong to those families and the wrapper user expects them.
///
/// The entire offending sequence is removed from the `ESC` introducer
/// through the string terminator (`BEL` or `ESC \`) inclusive. If a
/// terminator never arrives (truncated frame), the rest of the buffer is
/// dropped — better to swallow a keystroke than to leak a half-built OSC
/// into the PTY where the host terminal would still parse it.
fn sanitize_phone_input(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b && i + 1 < bytes.len() {
            let n = bytes[i + 1];
            // OSC=`]`, DCS=`P`, APC=`_`, PM=`^`, SOS=`X`. CSI=`[` and SS3=`O`
            // fall through to the `out.push(b)` branch on purpose.
            if matches!(n, b']' | b'P' | b'_' | b'^' | b'X') {
                let mut j = i + 2;
                while j < bytes.len() {
                    if bytes[j] == 0x07 {
                        j += 1;
                        break;
                    }
                    if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                        j += 2;
                        break;
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_phone_input;

    #[test]
    fn passes_through_printable_ascii() {
        let input = b"hello world\n";
        assert_eq!(sanitize_phone_input(input), input.to_vec());
    }

    #[test]
    fn preserves_csi_arrow_keys() {
        // ESC [ A (up), ESC [ B (down), ESC [ C (right), ESC [ D (left)
        let input = b"\x1b[A\x1b[B\x1b[C\x1b[D";
        assert_eq!(sanitize_phone_input(input), input.to_vec());
    }

    #[test]
    fn preserves_ss3_function_keys() {
        // ESC O P (F1), ESC O Q (F2), ESC O R (F3), ESC O S (F4)
        let input = b"\x1bOP\x1bOQ\x1bOR\x1bOS";
        assert_eq!(sanitize_phone_input(input), input.to_vec());
    }

    #[test]
    fn preserves_bracketed_paste_markers() {
        let input = b"\x1b[200~paste-body\x1b[201~";
        assert_eq!(sanitize_phone_input(input), input.to_vec());
    }

    #[test]
    fn strips_osc_52_clipboard_with_bel_terminator() {
        // OSC 52 ; c ; <base64> BEL — the classic clipboard hijack.
        let input = b"before\x1b]52;c;QUJD\x07after";
        assert_eq!(sanitize_phone_input(input), b"beforeafter".to_vec());
    }

    #[test]
    fn strips_osc_with_st_terminator() {
        // ESC \ (0x1b 0x5c) is the canonical String Terminator.
        let input = b"before\x1b]0;title\x1b\\after";
        assert_eq!(sanitize_phone_input(input), b"beforeafter".to_vec());
    }

    #[test]
    fn strips_dcs() {
        let input = b"a\x1bPq#sixel-payload\x1b\\b";
        assert_eq!(sanitize_phone_input(input), b"ab".to_vec());
    }

    #[test]
    fn strips_apc_pm_sos() {
        for introducer in [b'_', b'^', b'X'] {
            let mut input = vec![b'a', 0x1b, introducer];
            input.extend_from_slice(b"payload");
            input.extend_from_slice(b"\x1b\\");
            input.push(b'b');
            assert_eq!(
                sanitize_phone_input(&input),
                b"ab".to_vec(),
                "introducer 0x{:02x} should be stripped",
                introducer
            );
        }
    }

    #[test]
    fn truncated_osc_drops_remainder() {
        // No terminator before end-of-buffer: drop everything from ESC ].
        let input = b"safe\x1b]52;c;UNTERMINATED";
        assert_eq!(sanitize_phone_input(input), b"safe".to_vec());
    }

    #[test]
    fn lone_trailing_esc_preserved() {
        // A bare ESC at end-of-buffer has no introducer to classify — leave
        // it so an interactive ESC key (sent as a single byte, the terminator
        // arrives in the next frame) is still delivered. The wrapper PTY
        // line discipline will reassemble.
        let input = b"abc\x1b";
        assert_eq!(sanitize_phone_input(input), input.to_vec());
    }

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(sanitize_phone_input(b""), Vec::<u8>::new());
    }
}
