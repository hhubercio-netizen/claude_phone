use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    error::GatewayError,
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
        api_keys: vec![api_key.clone()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
        public_origin: None,
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

/// GatewayError contract: NO variant — current or future — may surface a raw
/// SessionToken or ApiKey through `Debug` / `Display`. Today none of the
/// variants carry a secret payload, so this test passes trivially. The point
/// is forward-looking: if a contributor ever adds e.g.
/// `InvalidToken(SessionToken)` and writes `#[error("bad token: {0}")]`, this
/// test breaks the build. SessionToken/ApiKey both redact their own `Debug`,
/// so the failure mode would be a `#[error(...)]` Display string built from
/// `token.as_str()` or similar.
#[test]
fn gateway_error_never_leaks_secrets_in_debug_or_display() {
    let token = SessionToken::generate();
    let api_key = ApiKey::generate();
    let token_str = token.as_str().to_string();
    let api_key_str = api_key.as_str().to_string();

    let variants: Vec<GatewayError> = vec![
        GatewayError::SessionNotFound,
        GatewayError::InvalidToken,
        GatewayError::InvalidApiKey,
        GatewayError::SessionTaken,
        GatewayError::Internal(anyhow::anyhow!("synthetic internal failure")),
        GatewayError::Io(std::io::Error::other("synthetic io failure")),
    ];

    for v in &variants {
        let dbg = format!("{:?}", v);
        let disp = format!("{}", v);
        assert!(
            !dbg.contains(&token_str),
            "GatewayError Debug leaked SessionToken in variant {dbg}"
        );
        assert!(
            !dbg.contains(&api_key_str),
            "GatewayError Debug leaked ApiKey in variant {dbg}"
        );
        assert!(
            !disp.contains(&token_str),
            "GatewayError Display leaked SessionToken in variant {disp}"
        );
        assert!(
            !disp.contains(&api_key_str),
            "GatewayError Display leaked ApiKey in variant {disp}"
        );
    }
}

/// Stronger sibling of the above: scans Debug output for ANY 43-char
/// base64url substring (the shape of both SessionToken and ApiKey). This
/// catches the case where a future variant carries a `String` payload that
/// happens to be a raw, un-typed token — at which point the typed-secret
/// redaction wouldn't fire because the value never wore the type.
#[test]
fn gateway_error_debug_contains_no_token_shaped_substring() {
    fn looks_like_secret(s: &str) -> bool {
        const LEN: usize = SessionToken::LEN; // == ApiKey::LEN == 43
        if s.len() < LEN {
            return false;
        }
        let bytes = s.as_bytes();
        for window in bytes.windows(LEN) {
            if window
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return true;
            }
        }
        false
    }

    let variants: Vec<GatewayError> = vec![
        GatewayError::SessionNotFound,
        GatewayError::InvalidToken,
        GatewayError::InvalidApiKey,
        GatewayError::SessionTaken,
        GatewayError::Internal(anyhow::anyhow!("synthetic internal failure")),
        GatewayError::Io(std::io::Error::other("synthetic io failure")),
    ];

    for v in &variants {
        let dbg = format!("{:?}", v);
        assert!(
            !looks_like_secret(&dbg),
            "GatewayError Debug looks like it might contain a secret-shaped substring: {dbg}"
        );
    }
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
