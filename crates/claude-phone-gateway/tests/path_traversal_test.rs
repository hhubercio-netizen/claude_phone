//! TM-INPUT.6 — forward-looking integration tests against path-traversal
//! attempts on the `/assets/*` static-file route.
//!
//! Built directly on top of `tokio::net::TcpStream` (NOT `reqwest`) so the
//! exact bytes sent on the wire are observable. Any HTTP client library
//! that normalises the request-target URI before sending would silently
//! mask the regression we are trying to prevent — `reqwest::get(".../assets/../etc/passwd")`
//! resolves the `..` on the client side and never gives the server a chance
//! to either accept or reject it.

use std::time::Duration;

use claude_phone_gateway::{
    config::{Environment, GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::ApiKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Spawn an isolated gateway with a known `assets/` layout. Writes a
/// recognisable canary file inside `assets/` so the `serves_legit_asset`
/// sanity check can prove the test harness wiring is correct (i.e. we
/// would observe a true 200 if traversal were broken).
async fn spawn_gateway() -> (u16, tempfile::TempDir) {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>")
        .expect("write index.html");
    let assets = static_dir.path().join("assets");
    std::fs::create_dir_all(&assets).expect("assets dir");
    std::fs::write(assets.join("canary.txt"), "canary-ok").expect("write canary");
    // Outside the assets dir but inside static_dir — a traversal that
    // escaped `assets/` could in principle reach this file. The test
    // asserts that it CANNOT.
    std::fs::write(static_dir.path().join("secret.txt"), "outside-assets")
        .expect("write secret outside assets");

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
    tokio::time::sleep(Duration::from_millis(50)).await;
    (port, static_dir)
}

/// Send a single raw HTTP/1.1 request and return (status_line, body) as
/// best-effort strings. Status line is the first CRLF-terminated line of
/// the response. Body is whatever follows the header block.
async fn raw_http_get(port: u16, raw_target: &str) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let req = format!("GET {raw_target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write");

    let mut buf = Vec::with_capacity(4096);
    // Bound the read so a regression that 200s + streams the whole filesystem
    // doesn't hang the test runner.
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .expect("read timeout")
        .expect("read");
    let text = String::from_utf8_lossy(&buf).to_string();
    let mut parts = text.splitn(2, "\r\n\r\n");
    let head = parts.next().unwrap_or("").to_string();
    let body = parts.next().unwrap_or("").to_string();
    let status_line = head.lines().next().unwrap_or("").to_string();
    (status_line, body)
}

#[tokio::test]
async fn serve_dir_rejects_dot_dot_traversal() {
    let (port, _tmp) = spawn_gateway().await;
    let (status, body) = raw_http_get(port, "/assets/../secret.txt").await;
    // tower-http canonicalizes & escapes the asset root; the response is
    // either 404 (not found inside assets/) or 400 (rejected outright).
    // Either is acceptable — what is NOT acceptable is 200 with the
    // outside-assets file body.
    assert!(
        status.contains("404") || status.contains("400"),
        "expected 4xx, got {status:?}"
    );
    assert!(
        !body.contains("outside-assets"),
        "outside-assets file leaked through traversal: body={body:?}"
    );
}

#[tokio::test]
async fn serve_dir_rejects_url_encoded_dot_dot_traversal() {
    let (port, _tmp) = spawn_gateway().await;
    let (status, body) = raw_http_get(port, "/assets/%2e%2e/secret.txt").await;
    assert!(
        !status.contains("200"),
        "URL-encoded traversal must NOT 200: got {status:?}"
    );
    assert!(
        !body.contains("outside-assets"),
        "URL-encoded traversal leaked outside-assets: body={body:?}"
    );
}

#[tokio::test]
async fn serve_dir_rejects_double_slash_traversal() {
    let (port, _tmp) = spawn_gateway().await;
    let (status, body) = raw_http_get(port, "/assets//../secret.txt").await;
    assert!(
        !status.contains("200"),
        "double-slash traversal must NOT 200: got {status:?}"
    );
    assert!(
        !body.contains("outside-assets"),
        "double-slash traversal leaked outside-assets: body={body:?}"
    );
}

#[tokio::test]
async fn serve_dir_serves_legit_asset() {
    let (port, _tmp) = spawn_gateway().await;
    let (status, body) = raw_http_get(port, "/assets/canary.txt").await;
    // Sanity: a legitimate asset path inside the directory must serve, so
    // the traversal-rejection tests above can't pass simply because the
    // route is broken.
    assert!(status.contains("200"), "expected 200, got {status:?}");
    assert!(
        body.contains("canary-ok"),
        "expected canary content, got body={body:?}"
    );
}
