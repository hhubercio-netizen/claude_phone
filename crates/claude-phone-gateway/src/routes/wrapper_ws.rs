use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};

use crate::auth::verify_api_key;
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
    State(state): State<WrapperWsState>,
) -> Response {
    if let Some(expected) = state.public_origin.as_deref() {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            if origin != expected {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: WrapperWsState) {
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
        send_error(
            &mut socket,
            ErrorCode::InvalidApiKey,
            "unknown api key".into(),
        )
        .await;
        return;
    }

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
    let outgoing_task = tokio::spawn(async move {
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
