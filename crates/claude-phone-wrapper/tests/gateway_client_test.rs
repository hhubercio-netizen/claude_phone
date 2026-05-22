use std::sync::Arc;

use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::{routing::any, Router};
use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};
use claude_phone_shared::{ApiKey, SessionToken};
use claude_phone_wrapper::gateway_client::{GatewayClient, GatewayClientConfig};
use futures::StreamExt;
use tokio::sync::Mutex;

#[allow(clippy::enum_variant_names)]
enum FakeBehavior {
    SendServerHello,
    SendError,
    SendBinary,
}

async fn run_fake_gateway(behavior: Arc<Mutex<FakeBehavior>>) -> u16 {
    let port = portpicker::pick_unused_port().expect("free port");
    let behavior_for_route = behavior.clone();
    let app = Router::new().route(
        "/api/wrapper",
        any(move |ws: WebSocketUpgrade| {
            let behavior = behavior_for_route.clone();
            async move { ws.on_upgrade(move |socket| handle_socket(socket, behavior)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    port
}

async fn handle_socket(mut socket: WebSocket, behavior: Arc<Mutex<FakeBehavior>>) {
    let _hello = socket.next().await;
    let beh = behavior.lock().await;
    let response = match *beh {
        FakeBehavior::SendServerHello => AxumMessage::Text(
            serde_json::to_string(&ControlMessage::ServerHello(ServerHello {
                session_id: "test-session".into(),
                peer_connected: false,
            }))
            .unwrap(),
        ),
        FakeBehavior::SendError => AxumMessage::Text(
            serde_json::to_string(&ControlMessage::Error(ErrorMessage {
                code: ErrorCode::InvalidApiKey,
                message: "invalid api_key".into(),
            }))
            .unwrap(),
        ),
        FakeBehavior::SendBinary => AxumMessage::Binary(vec![1, 2, 3]),
    };
    let _ = socket.send(response).await;
}

#[tokio::test]
async fn happy_path_returns_session_id() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendServerHello));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let client = GatewayClient::connect(config).await.expect("connect");
    assert_eq!(client.session_id(), "test-session");
}

#[tokio::test]
async fn error_response_returns_err() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendError));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn binary_first_frame_returns_err() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendBinary));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn unreachable_host_returns_err() {
    let config = GatewayClientConfig {
        url: "ws://127.0.0.1:1/api/wrapper".into(),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
}
