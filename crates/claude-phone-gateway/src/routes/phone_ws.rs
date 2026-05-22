use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use futures::{SinkExt, StreamExt};

use claude_phone_shared::protocol::{
    ControlMessage, ErrorCode, ErrorMessage, PeerStatus, ServerHello,
};
use claude_phone_shared::SessionToken;

use crate::session::{Frame, SessionRegistry};

#[derive(Clone)]
pub struct PhoneWsState {
    pub registry: Arc<SessionRegistry>,
}

pub async fn handler(
    ws: WebSocketUpgrade,
    Path(token_str): Path<String>,
    State(state): State<PhoneWsState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, token_str))
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
    if socket
        .send(Message::Text(serde_json::to_string(&server_hello).unwrap()))
        .await
        .is_err()
    {
        return;
    }

    let peer_up = ControlMessage::PeerStatus(PeerStatus { connected: true });
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(serde_json::to_string(&peer_up).unwrap()))
        .await;

    tracing::info!(session_id = %session_id, "phone attached");

    let (mut sink, mut stream) = socket.split();
    let mut rx_from_wrapper = handle.rx;
    let to_wrapper = handle.session.to_wrapper.clone();

    let outgoing_task = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let Ok(msg) = msg else { break };
            let frame = match msg {
                Message::Binary(b) => Frame::Binary(b),
                Message::Text(t) => Frame::Text(t),
                Message::Close(_) => break,
                _ => continue,
            };
            if to_wrapper.send(frame).await.is_err() {
                break;
            }
        }
    });

    let incoming_task = tokio::spawn(async move {
        while let Some(frame) = rx_from_wrapper.recv().await {
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

    {
        let mut slot = handle.session.to_phone.lock().await;
        *slot = None;
    }
    let peer_down = ControlMessage::PeerStatus(PeerStatus { connected: false });
    let _ = handle
        .session
        .to_wrapper
        .send(Frame::Text(serde_json::to_string(&peer_down).unwrap()))
        .await;
    tracing::info!(session_id = %session_id, "phone detached");
}

async fn send_error(socket: &mut WebSocket, code: ErrorCode, message: String) {
    let err = ControlMessage::Error(ErrorMessage { code, message });
    let _ = socket
        .send(Message::Text(serde_json::to_string(&err).unwrap()))
        .await;
}
