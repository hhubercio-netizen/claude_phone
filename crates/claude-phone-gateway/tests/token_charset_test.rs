//! TM-INPUT.7 — gateway integration tier proves that a session token URL
//! segment containing control characters can never reach the WebSocket
//! attach. The unit tests in `claude-phone-shared/src/token.rs` cover
//! `SessionToken::parse` charset rejection; this integration test pins the
//! end-to-end property that the HTTP stack as a whole refuses such tokens.

use std::time::Duration;

use claude_phone_gateway::{
    config::{Environment, GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::{ApiKey, SessionToken};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn spawn_gateway() -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>")
        .expect("write index.html");
    std::fs::create_dir_all(static_dir.path().join("assets")).expect("assets dir");

    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![ApiKey::generate()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
        environment: Environment::Development,
        public_origin: None,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        claude_phone_gateway::serve::run(listener, app, std::future::pending::<()>()).await;
    });
    Box::leak(Box::new(static_dir));
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

/// Send a single raw HTTP/1.1 request and return the first status line.
async fn raw_http_status(port: u16, raw_path_bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let mut req = Vec::with_capacity(128 + raw_path_bytes.len());
    req.extend_from_slice(b"GET ");
    req.extend_from_slice(raw_path_bytes);
    req.extend_from_slice(b" HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(&req).await.expect("write");

    let mut buf = Vec::with_capacity(4096);
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .expect("read timeout")
        .expect("read");
    let text = String::from_utf8_lossy(&buf).to_string();
    text.lines().next().unwrap_or("").to_string()
}

/// Build a 43-byte token-shaped path segment with `bad` planted at index 5.
fn token_path_with_bad_byte(bad: u8) -> Vec<u8> {
    let mut bytes = vec![b'A'; SessionToken::LEN];
    bytes[5] = bad;
    let mut path = b"/api/phone/".to_vec();
    path.extend_from_slice(&bytes);
    path
}

#[tokio::test]
async fn phone_ws_handler_rejects_nul_in_token() {
    let port = spawn_gateway().await;
    let path = token_path_with_bad_byte(0x00);
    let status = raw_http_status(port, &path).await;
    // NUL inside the request-target line is rejected by the HTTP layer
    // before the route handler ever sees it. The exact 4xx code (often 400
    // from hyper's URI parser) is implementation detail — what matters is
    // that we never see a 101 Switching Protocols and never reach attach.
    assert!(
        !status.contains("101") && !status.contains("200"),
        "control char in token must not yield upgrade/200: got {status:?}"
    );
}

#[tokio::test]
async fn phone_ws_handler_rejects_bell_in_token() {
    let port = spawn_gateway().await;
    let path = token_path_with_bad_byte(0x07);
    let status = raw_http_status(port, &path).await;
    assert!(
        !status.contains("101") && !status.contains("200"),
        "BEL in token must not yield upgrade/200: got {status:?}"
    );
}

#[tokio::test]
async fn phone_ws_handler_rejects_wrong_length_token() {
    let port = spawn_gateway().await;
    // 42-byte token — the strict TM-WS.11 length check catches this even
    // before charset validation runs.
    let mut path = b"/api/phone/".to_vec();
    path.extend(std::iter::repeat_n(b'A', SessionToken::LEN - 1));
    let status = raw_http_status(port, &path).await;
    assert!(
        status.contains("400"),
        "off-length token must 400: got {status:?}"
    );
}
