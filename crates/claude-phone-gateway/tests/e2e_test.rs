use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::{
    protocol::{ControlMessage, PhoneHello, WrapperHello},
    ApiKey, SessionToken,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey) -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>")
        .expect("write index.html");
    std::fs::create_dir_all(static_dir.path().join("assets")).expect("assets dir");

    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![api_key.as_str().to_string()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Box::leak(Box::new(static_dir));
    port
}

#[tokio::test]
async fn wrapper_and_phone_round_trip() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut wrapper_ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    wrapper_ws
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let server_hello = wrapper_ws.next().await.unwrap().unwrap();
    assert!(matches!(server_hello, Message::Text(_)));

    let (mut phone_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");

    let p_hello = ControlMessage::PhoneHello(PhoneHello {
        token: token.clone(),
        cols: 40,
        rows: 80,
        user_agent: None,
    });
    phone_ws
        .send(Message::Text(serde_json::to_string(&p_hello).unwrap()))
        .await
        .unwrap();
    let _phone_server_hello = phone_ws.next().await.unwrap().unwrap();

    wrapper_ws
        .send(Message::Binary(b"hello phone".to_vec()))
        .await
        .unwrap();
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..3 {
        let msg = phone_ws.next().await.unwrap().unwrap();
        if let Message::Binary(b) = msg {
            got = Some(b);
            break;
        }
    }
    assert_eq!(got.as_deref(), Some(&b"hello phone"[..]));

    phone_ws
        .send(Message::Binary(b"hi wrapper".to_vec()))
        .await
        .unwrap();
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..3 {
        let msg = wrapper_ws.next().await.unwrap().unwrap();
        if let Message::Binary(b) = msg {
            got = Some(b);
            break;
        }
    }
    assert_eq!(got.as_deref(), Some(&b"hi wrapper"[..]));
}

#[tokio::test]
async fn wrapper_rejects_bad_api_key() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: ApiKey::generate(),
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
        other => panic!("expected text, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::Error(_)));
}

#[tokio::test]
async fn phone_rejects_unknown_token() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        SessionToken::generate().as_str()
    ))
    .await
    .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::Error(_)));
}
