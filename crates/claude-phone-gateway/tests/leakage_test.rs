use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::{
    protocol::{ControlMessage, WrapperHello},
    ApiKey, SessionToken,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey) -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>").unwrap();
    std::fs::create_dir_all(static_dir.path().join("assets")).unwrap();
    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![api_key.as_str().to_string()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
    };
    let app = build_app(&config).unwrap();
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Box::leak(Box::new(static_dir));
    port
}

#[tokio::test]
async fn error_response_does_not_echo_api_key() {
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_key = ApiKey::generate();
    let bad_key_str = bad_key.as_str().to_string();
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: bad_key,
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };

    assert!(
        !text.contains(&bad_key_str),
        "error response leaked api_key value: {text}"
    );
}

#[tokio::test]
async fn error_response_does_not_echo_token() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_token = SessionToken::generate();
    let bad_token_str = bad_token.as_str().to_string();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{bad_token_str}"
    ))
    .await
    .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        Message::Close(frame) => format!("{:?}", frame),
        other => panic!("expected text or close, got {other:?}"),
    };

    assert!(
        !text.contains(&bad_token_str),
        "phone error response leaked token value: {text}"
    );
}

#[tokio::test]
#[tracing_test::traced_test]
async fn tracing_does_not_leak_token_or_api_key_on_failure() {
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_key = ApiKey::generate();
    let bad_key_str = bad_key.as_str().to_string();
    let bad_token = SessionToken::generate();
    let bad_token_str = bad_token.as_str().to_string();

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: bad_key,
        token: bad_token,
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let _ = ws.next().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        !logs_contain(&bad_key_str),
        "tracing leaked api_key on registration failure"
    );
    assert!(
        !logs_contain(&bad_token_str),
        "tracing leaked token on registration failure"
    );
}
