use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};

use crate::auth::verify_api_key;
use crate::session::{Frame, SessionRegistry};

#[derive(Clone)]
pub struct WrapperWsState {
    pub registry: Arc<SessionRegistry>,
    pub allowed_keys: Arc<Vec<claude_phone_shared::ApiKey>>,
}

pub async fn handler(ws: WebSocketUpgrade, State(state): State<WrapperWsState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
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
    if socket
        .send(Message::Text(serde_json::to_string(&server_hello).unwrap()))
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
    let outgoing_task = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let Ok(msg) = msg else { break };
            let frame = match msg {
                Message::Binary(b) => Frame::Binary(b),
                Message::Text(t) => Frame::Text(t),
                Message::Close(_) => break,
                _ => continue,
            };
            let slot = session_outgoing.to_phone.lock().await;
            if let Some(tx) = slot.as_ref() {
                let _ = tx.send(frame).await;
            }
        }
    });

    let incoming_task = tokio::spawn(async move {
        while let Some(frame) = rx_from_phone.recv().await {
            let msg = match frame {
                Frame::Binary(b) => Message::Binary(b),
                Frame::Text(t) => Message::Text(t),
            };
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(outgoing_task, incoming_task);
    state.registry.remove(&token);
    tracing::info!(session_id = %session_id, "wrapper detached");
}

async fn recv_hello(socket: &mut WebSocket) -> Option<ControlMessage> {
    let msg = socket.recv().await?.ok()?;
    let Message::Text(t) = msg else { return None };
    serde_json::from_str(&t).ok()
}

async fn send_error(socket: &mut WebSocket, code: ErrorCode, message: String) {
    let err = ControlMessage::Error(ErrorMessage { code, message });
    let _ = socket
        .send(Message::Text(serde_json::to_string(&err).unwrap()))
        .await;
}
