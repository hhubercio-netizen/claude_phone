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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey) -> u16 {
    spawn_test_gateway_with_origin(api_key, None).await
}

async fn spawn_test_gateway_with_origin(api_key: ApiKey, public_origin: Option<String>) -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>")
        .expect("write index.html");
    std::fs::create_dir_all(static_dir.path().join("assets")).expect("assets dir");

    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![api_key.clone()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
        public_origin,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        // TM-RATE.1/.9 — exercise the same serve loop the binary uses so
        // GovernorLayer gets ConnectInfo and slow-loris timeout fires.
        // axum::serve here would skip both and let regressions land green.
        claude_phone_gateway::serve::run(listener, app, std::future::pending::<()>()).await;
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

// Build a tungstenite ws:// request with an explicit Origin header. Browsers
// always send Origin; non-browser clients (test clients, server-side bridges)
// may not. The gateway only enforces equality when Origin is present AND
// public_origin is configured.
fn ws_request_with_origin(
    url: &str,
    origin: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut()
        .insert("origin", origin.parse().expect("origin header value"));
    req
}

#[tokio::test]
async fn phone_ws_accepts_matching_origin() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let expected_origin = "https://claude-phone.pl".to_string();
    let port = spawn_test_gateway_with_origin(api_key.clone(), Some(expected_origin.clone())).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Register wrapper so the phone token is known.
    let (mut wrapper_ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
    });
    wrapper_ws
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let _ = wrapper_ws.next().await.unwrap().unwrap();

    let req = ws_request_with_origin(
        &format!("ws://127.0.0.1:{port}/api/phone/{}", token.as_str()),
        &expected_origin,
    );
    let (mut phone_ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("phone connect with matching origin should succeed");

    let resp = phone_ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::ServerHello(_)));
}

#[tokio::test]
async fn phone_ws_rejects_wrong_origin() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway_with_origin(
        api_key.clone(),
        Some("https://claude-phone.pl".to_string()),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Register wrapper so the token would otherwise be valid.
    let (mut wrapper_ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
    });
    wrapper_ws
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let _ = wrapper_ws.next().await.unwrap().unwrap();

    let req = ws_request_with_origin(
        &format!("ws://127.0.0.1:{port}/api/phone/{}", token.as_str()),
        "https://evil.example.com",
    );
    let err = tokio_tungstenite::connect_async(req)
        .await
        .expect_err("mismatched Origin must be rejected before upgrade");
    let msg = format!("{err}");
    assert!(
        msg.contains("403") || msg.to_lowercase().contains("forbidden"),
        "expected 403/forbidden, got: {msg}"
    );
}

#[tokio::test]
async fn phone_ws_rejects_missing_origin_when_public_origin_set() {
    // TM-WS.3 — phone_ws is browser-served (the page at /s/:token opens a
    // same-origin WebSocket), so a legitimate browser always sends Origin.
    // A missing Origin when public_origin is configured is either a
    // non-browser client probing with a stolen token, or a stripped
    // header. Both deserve 403. Wrapper_ws keeps the CLI-client carveout
    // (covered by wrapper_ws_accepts_matching_origin and the websocket.rs
    // wrapper_ws_allows_missing_origin_even_when_public_origin_configured).
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway_with_origin(
        api_key.clone(),
        Some("https://claude-phone.pl".to_string()),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // tokio_tungstenite's plain URL form doesn't add Origin.
    let err = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect_err("phone connect without Origin must be rejected when public_origin is set");
    let status = match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status(),
        other => panic!("expected Http error, got: {other:?}"),
    };
    assert_eq!(
        status.as_u16(),
        403,
        "TM-WS.3: missing Origin on phone_ws must yield 403, got {status:?}"
    );
}

#[tokio::test]
async fn wrapper_ws_rejects_wrong_origin() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway_with_origin(
        api_key.clone(),
        Some("https://claude-phone.pl".to_string()),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let req = ws_request_with_origin(
        &format!("ws://127.0.0.1:{port}/api/wrapper"),
        "https://evil.example.com",
    );
    let err = tokio_tungstenite::connect_async(req)
        .await
        .expect_err("mismatched Origin on wrapper WS must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("403") || msg.to_lowercase().contains("forbidden"),
        "expected 403/forbidden, got: {msg}"
    );
}

#[tokio::test]
async fn wrapper_ws_accepts_matching_origin() {
    let api_key = ApiKey::generate();
    let expected_origin = "https://claude-phone.pl".to_string();
    let port = spawn_test_gateway_with_origin(api_key.clone(), Some(expected_origin.clone())).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let req = ws_request_with_origin(
        &format!("ws://127.0.0.1:{port}/api/wrapper"),
        &expected_origin,
    );
    let (mut wrapper_ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("matching Origin on wrapper WS should succeed");

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    });
    wrapper_ws
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let resp = wrapper_ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::ServerHello(_)));
}

#[tokio::test]
async fn wrapper_ws_hello_timeout_drops_idle_socket() {
    // Slow-loris defense: a wrapper client that completes the WS upgrade but
    // never sends a WrapperHello must have its socket reaped after
    // HELLO_TIMEOUT (10s in production). To keep the test fast we don't wait
    // the full 10s — we connect, never send hello, and assert the server
    // closes the stream within a bounded window (16s envelope). The negative
    // assertion (server holds the socket forever) would obviously hang.
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut wrapper_ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");

    // Don't send anything. Wait for the server to give up.
    let closed = tokio::time::timeout(Duration::from_secs(16), async {
        loop {
            match wrapper_ws.next().await {
                None => break,
                Some(Err(_)) => break,
                Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "gateway must close idle wrapper WS within HELLO_TIMEOUT envelope"
    );
}

#[tokio::test]
async fn phone_ws_rejects_malformed_token_length() {
    // Strict length check: anything other than SessionToken::LEN gets a 400
    // before we even allocate the WS upgrade machinery. This is the cheap
    // pre-filter that keeps off-shape probe strings from touching session code.
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 10 chars is below SessionToken::LEN (43).
    let too_short = "abcdefghij";
    let err =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/phone/{too_short}"))
            .await
            .expect_err("malformed-length token must be rejected with 400");
    let msg = format!("{err}");
    assert!(
        msg.contains("400") || msg.to_lowercase().contains("bad request"),
        "expected 400/bad request, got: {msg}"
    );
}
