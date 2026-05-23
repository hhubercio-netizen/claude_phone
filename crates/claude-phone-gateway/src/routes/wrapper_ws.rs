use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};

use crate::auth::verify_api_key;
use crate::rate_limit::{AuthRateLimiter, ConnRateLimiter, GW_TO_PHONE_MSG_PER_SEC};
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

    let session_outgoing = session.clone();
    let cancel_outgoing = session.cancel.clone();
    let session_for_flood = session.clone();
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
                    let Some(Ok(msg)) = msg else { break };
                    let frame = match msg {
                        Message::Binary(b) => Frame::Binary(b),
                        Message::Text(t) => Frame::Text(t),
                        Message::Close(_) => break,
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
    let incoming_task = tokio::spawn(async move {
        // Periodically send a Ping so NAT/Cloudflare proxies don't tear the
        // idle WebSocket down between bursts of PTY output. Wrapper-side WS
        // also handles incoming Pong silently via the `_` branch.
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
                    if sink.send(msg).await.is_err() { break; }
                }
                _ = keepalive.tick() => {
                    if sink.send(Message::Ping(Vec::new())).await.is_err() {
                        break;
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
