use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};

use crate::auth::verify_api_key;
use crate::rate_limit::{
    AuthRateLimiter, ConnRateLimiter, GW_TO_PHONE_MSG_PER_SEC, PONG_DEADLINE, SINK_SEND_TIMEOUT,
};
use crate::session::{Frame, SessionRegistry};

#[derive(Clone)]
pub struct WrapperWsState {
    pub registry: Arc<SessionRegistry>,
    pub allowed_keys: Arc<Vec<claude_phone_shared::ApiKey>>,
    /// When `Some`, browser-initiated WSes whose `Origin` header doesn't
    /// equal this value are rejected with 403. Mirrors phone_ws — defense in
    /// depth against CSWSH where a malicious page running in a victim's
    /// browser tries to mount a wrapper session.
    pub public_origin: Option<String>,
    /// TM-RATE.2 — per-IP auth-failure tracker. Shared across all wrapper
    /// upgrades so failures accumulate against a single IP regardless of
    /// concurrent attempts.
    pub auth_rate_limiter: AuthRateLimiter,
}

/// See phone_ws::MAX_WS_MESSAGE_BYTES — wrapper carries PTY chunks (8KB) and
/// JSON control messages (small). 64KB caps DoS surface from a malicious peer.
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;

/// Hard wall-clock budget for the wrapper to deliver its `WrapperHello` after
/// the WebSocket upgrade completes. Without this a slow-loris client could
/// hold sockets open indefinitely, costing file descriptors and tying up
/// axum's accept queue. Real wrappers send hello within milliseconds.
const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<WrapperWsState>,
) -> Response {
    // TM-RATE.2 — short-circuit upgrade for IPs that have tripped the
    // auth-failure lockout. 429 communicates "try again later" without
    // revealing whether the API key would otherwise have been valid.
    if state.auth_rate_limiter.is_locked(peer.ip()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    if let Some(expected) = state.public_origin.as_deref() {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            if origin != expected {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state, peer))
}

async fn handle_socket(mut socket: WebSocket, state: WrapperWsState, peer: SocketAddr) {
    let hello = match recv_hello(&mut socket).await {
        Some(h) => h,
        None => return,
    };

    let (api_key, token) = match hello {
        ControlMessage::WrapperHello(h) => (h.api_key, h.token),
        _ => {
            send_error(
                &mut socket,
                ErrorCode::ProtocolViolation,
                "expected wrapper_hello".into(),
            )
            .await;
            return;
        }
    };

    if !verify_api_key(&api_key, &state.allowed_keys) {
        // TM-RATE.2 — record the failure BEFORE responding so a brute-forcer
        // hitting in rapid succession ratchets the counter on every miss.
        // The 4.2 sub-spec also emits a structured auth-failure log; the
        // counter side-effect lives here, the log line is upstream — no
        // duplication.
        state.auth_rate_limiter.record_failure(peer.ip());
        send_error(
            &mut socket,
            ErrorCode::InvalidApiKey,
            "unknown api key".into(),
        )
        .await;
        return;
    }
    // TM-RATE.2 — clear escalation count once we've confirmed the operator
    // owns a valid key. Otherwise an honest user who fat-fingered earlier
    // could find themselves locked out hours later.
    state.auth_rate_limiter.record_success(peer.ip());

    let handle = match state.registry.register_wrapper(token.clone()).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = ?e, "wrapper registration failed");
            send_error(&mut socket, ErrorCode::SessionTaken, e.to_string()).await;
            return;
        }
    };

    let session_id = handle.session.id.clone();
    let server_hello = ControlMessage::ServerHello(ServerHello {
        session_id: session_id.clone(),
        peer_connected: false,
    });
    // TM-CODE.3: ServerHello is a derive(Serialize) struct of owned Strings
    // and primitives — serde_json::to_string is infallible in practice.
    if socket
        .send(Message::Text(
            serde_json::to_string(&server_hello).expect("ServerHello serializes (static struct)"),
        ))
        .await
        .is_err()
    {
        state.registry.remove(&token);
        return;
    }

    tracing::info!(session_id = %session_id, "wrapper attached");

    let (mut sink, mut stream) = socket.split();
    let mut rx_from_phone = handle.rx;
    let session = handle.session.clone();

    // TM-RATE.7 — shared "millis since this socket opened" reference for the
    // no-pong watchdog. Both tasks see the same `start`; `last_pong_ms`
    // stamps a Relaxed write on every incoming Pong and a Relaxed read on
    // every keepalive tick. AtomicU64 keeps the watchdog lock-free.
    let socket_start = Instant::now();
    let last_pong_ms = Arc::new(AtomicU64::new(0));

    let session_outgoing = session.clone();
    let cancel_outgoing = session.cancel.clone();
    let session_for_flood = session.clone();
    let last_pong_outgoing = last_pong_ms.clone();
    let outgoing_task = tokio::spawn(async move {
        // TM-RATE.3 — per-connection sliding-window cap. Wrapper→phone is the
        // PTY direction so bursts during `claude` output are expected. Cap at
        // GW_TO_PHONE_MSG_PER_SEC (1000/s). A peer exceeding this is either
        // malfunctioning or hostile; either way we close the session rather
        // than absorb unbounded traffic.
        let mut conn_rate = ConnRateLimiter::new(GW_TO_PHONE_MSG_PER_SEC);
        loop {
            let cancelled = cancel_outgoing.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                msg = stream.next() => {
                    // TM-RATE.7: producer-side break must propagate cancel
                    // so `incoming_task` wakes up immediately. Without this
                    // the `join!` waits until the keepalive watchdog notices
                    // the dead socket (up to 30 s + ping timeout), pinning
                    // the session slot in `state.registry` well past the
                    // moment the peer is actually gone — slow drain of
                    // `max_sessions` under a churning peer pattern.
                    let Some(Ok(msg)) = msg else {
                        cancel_outgoing.cancel();
                        break;
                    };
                    let frame = match msg {
                        Message::Binary(b) => Frame::Binary(b),
                        Message::Text(t) => Frame::Text(t),
                        Message::Close(_) => {
                            cancel_outgoing.cancel();
                            break;
                        }
                        // TM-RATE.7: stamp last_pong on every received Pong.
                        // The keepalive in incoming_task reads this to decide
                        // whether the peer is still reachable.
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
                            cap = GW_TO_PHONE_MSG_PER_SEC,
                            "TM-RATE.3: wrapper exceeded per-connection msg rate, closing"
                        );
                        // Cancelling the session also tears the phone side
                        // down — a flooding wrapper invalidates the session.
                        session_for_flood.cancel.cancel();
                        break;
                    }
                    // Snapshot the sender under the lock, then release before
                    // the potentially-blocking send. If no phone is attached,
                    // binary frames go to the replay buffer; text frames are
                    // dropped (transient control signals that would be
                    // confusing to replay).
                    let sender_opt = {
                        let mut slot = session_outgoing.to_phone.lock().await;
                        let s = slot.sender();
                        if s.is_none() {
                            if let Frame::Binary(bytes) = &frame {
                                slot.push_buffered(bytes.clone());
                            }
                        }
                        s
                    };
                    if let Some(tx) = sender_opt {
                        let _ = tx.send(frame).await;
                    }
                }
            }
        }
    });

    let cancel_incoming = session.cancel.clone();
    let last_pong_incoming = last_pong_ms.clone();
    let incoming_task = tokio::spawn(async move {
        // Periodically send a Ping so NAT/Cloudflare proxies don't tear the
        // idle WebSocket down between bursts of PTY output. The reply Pong
        // (handled in outgoing_task) stamps `last_pong_incoming`; this
        // task checks the age on every tick (TM-RATE.7).
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(30));
        keepalive.tick().await; // skip immediate tick

        loop {
            let cancelled = cancel_incoming.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                frame = rx_from_phone.recv() => {
                    let Some(frame) = frame else { break };
                    let msg = match frame {
                        Frame::Binary(b) => Message::Binary(b),
                        Frame::Text(t) => Message::Text(t),
                    };
                    // TM-RATE.6: bounded wait so a slow-reading peer can't
                    // pin the writer forever. timeout()->Err is "stalled",
                    // Ok(Err) is "socket dead"; both are terminal. On stall
                    // we cancel the session so the outgoing_task tears down
                    // too — a half-closed socket still costs an FD.
                    match tokio::time::timeout(SINK_SEND_TIMEOUT, sink.send(msg)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            cancel_incoming.cancel();
                            break;
                        }
                        Err(_) => {
                            tracing::warn!(
                                timeout_secs = SINK_SEND_TIMEOUT.as_secs(),
                                "TM-RATE.6: wrapper sink send stalled, closing"
                            );
                            cancel_incoming.cancel();
                            break;
                        }
                    }
                }
                _ = keepalive.tick() => {
                    // TM-RATE.7: check elapsed since last Pong BEFORE sending
                    // the next Ping. If the peer hasn't pong'd within
                    // PONG_DEADLINE the socket is silently dead; cancel and
                    // reclaim resources.
                    let now_ms = socket_start.elapsed().as_millis() as u64;
                    let last_ms = last_pong_incoming.load(Ordering::Relaxed);
                    let age_ms = now_ms.saturating_sub(last_ms);
                    if age_ms > PONG_DEADLINE.as_millis() as u64 {
                        tracing::warn!(
                            age_ms,
                            deadline_ms = PONG_DEADLINE.as_millis() as u64,
                            "TM-RATE.7: wrapper no-pong deadline exceeded, closing"
                        );
                        cancel_incoming.cancel();
                        break;
                    }
                    // TM-RATE.6: same bound on the keepalive write — a peer
                    // that won't accept a 0-byte ping isn't accepting anything.
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
    state.registry.remove(&token);
    tracing::info!(session_id = %session_id, "wrapper detached");
}

async fn recv_hello(socket: &mut WebSocket) -> Option<ControlMessage> {
    // Bounded wait so an idle peer can't pin a socket FD forever. timeout()
    // returns Err on elapse and Ok(None) when the stream closes cleanly —
    // both are treated as "no hello, hang up" here.
    let msg = tokio::time::timeout(HELLO_TIMEOUT, socket.recv())
        .await
        .ok()??
        .ok()?;
    let Message::Text(t) = msg else { return None };
    serde_json::from_str(&t).ok()
}

async fn send_error(socket: &mut WebSocket, code: ErrorCode, message: String) {
    let err = ControlMessage::Error(ErrorMessage { code, message });
    // TM-CODE.3: ErrorMessage is a derive(Serialize) struct of an enum tag
    // and a String — serde_json::to_string is infallible in practice.
    let _ = socket
        .send(Message::Text(
            serde_json::to_string(&err).expect("ErrorMessage serializes (static struct)"),
        ))
        .await;
}
