use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{
    ControlMessage, ErrorCode, ErrorMessage, PeerStatus, ServerHello,
};
use claude_phone_shared::SessionToken;

use crate::session::{Frame, SessionRegistry};

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

pub async fn handler(
    ws: WebSocketUpgrade,
    Path(token_str): Path<String>,
    headers: HeaderMap,
    State(state): State<PhoneWsState>,
) -> Response {
    // Strict equality on token length — anything else is malformed and we
    // refuse to even allocate the upgrade. The previous 8..=64 band let
    // off-shape strings reach SessionToken::parse() unnecessarily.
    if token_str.len() != SessionToken::LEN {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Defense-in-depth Origin check. Only fires when the deployer set
    // `public_origin` in gateway.toml AND the client sent an `Origin`
    // header (browsers always do; non-browser clients may not).
    if let Some(expected) = state.public_origin.as_deref() {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            if origin != expected {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state, token_str))
}

async fn handle_socket(mut socket: WebSocket, state: PhoneWsState, token_str: String) {
    let token = match SessionToken::parse(&token_str) {
        Ok(t) => t,
        Err(_) => {
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

    let cancel_outgoing = cancel.clone();
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
                    if to_wrapper.send(frame).await.is_err() { break; }
                }
            }
        }
    });

    let cancel_incoming = cancel.clone();
    let incoming_task = tokio::spawn(async move {
        // 30s server-initiated Ping keeps the phone's WebSocket alive across
        // NAT and Cloudflare's idle-connection drop (~100s). Pong replies
        // arrive through the same socket; axum dispatches them silently.
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
