//! Forward-looking integration tests for WebSocket-specific hardening.
//!
//! Sub-spec 4.13 lands a row of tests that pin guard behaviour on both
//! the wrapper and phone WS routes. They all drive `serve::run_with` so
//! a refactor that drops the guards is caught by CI, not by production.
//!
//! Test groups landed per commit:
//! - Commit 1 (this file's first revision): TM-WS.3 fail-closed missing Origin.
//! - Commit 3: TM-WS.8 / TM-WS.12 negative-assertion (compression / subprotocol).
//! - Commit 4: TM-WS.7 / TM-WS.10 asymmetry pins.

use std::time::Duration;

use claude_phone_gateway::{
    config::{Environment, GatewayConfig, LogFormat},
    http::build_app,
    serve,
};
use claude_phone_shared::{ApiKey, SessionToken};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const EXPECTED_ORIGIN: &str = "https://phone.example";

/// Spawn a gateway on a free port with the production serve loop. The
/// optional `public_origin` arg lets each TM-WS.* test pick the policy
/// branch it wants to exercise: `Some(...)` enforces Origin, `None`
/// disables the gate (dev / pre-production).
async fn spawn_gateway(public_origin: Option<String>) -> (u16, ApiKey) {
    let api_key = ApiKey::generate();
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
        environment: Environment::Development,
        public_origin,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        serve::run_with(
            listener,
            app,
            std::future::pending::<()>(),
            serve::HEADER_READ_TIMEOUT,
            Duration::from_secs(1),
        )
        .await;
    });
    // tempdir must outlive the spawned task or ServeDir will 404.
    Box::leak(Box::new(static_dir));

    tokio::time::sleep(Duration::from_millis(50)).await;
    (port, api_key)
}

/// Build a WS client `Request` without an `Origin` header. tungstenite
/// does not add `Origin` by default, so a plain `into_client_request` is
/// the "no Origin" case — see `tokio-tungstenite` handshake builder
/// (`tungstenite::handshake::client::generate_request`) which sets only
/// Host/Upgrade/Connection/Sec-WebSocket-{Key,Version}.
fn ws_request_no_origin(url: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    url.into_client_request().expect("ws client request")
}

fn ws_request_with_origin(
    url: &str,
    origin: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut req = ws_request_no_origin(url);
    req.headers_mut().insert(
        "origin",
        HeaderValue::from_str(origin).expect("origin header value"),
    );
    req
}

/// Pull the HTTP status off a tungstenite `Error::Http` — the upgrade
/// failure path. Any other error variant is a test bug (network down,
/// TLS handshake on a plain socket, etc.) and we panic with context.
fn expect_http_status(
    err: tokio_tungstenite::tungstenite::Error,
) -> tokio_tungstenite::tungstenite::http::StatusCode {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status(),
        other => panic!("expected Http error, got: {other:?}"),
    }
}

/// TM-WS.3 — Phone WS MUST refuse the upgrade with 403 when
/// `public_origin` is configured and the client omits the `Origin`
/// header. Browsers always send Origin on a same-origin WS; absence is
/// either a non-browser client or a stripped header — both deserve 403.
#[tokio::test]
async fn phone_ws_rejects_missing_origin_when_public_origin_configured() {
    let (port, _key) = spawn_gateway(Some(EXPECTED_ORIGIN.to_string())).await;
    let token = SessionToken::generate();
    let url = format!("ws://127.0.0.1:{port}/api/phone/{}", token.as_str());

    let err = tokio_tungstenite::connect_async(ws_request_no_origin(&url))
        .await
        .expect_err("missing Origin must be rejected when public_origin is configured");
    assert_eq!(
        expect_http_status(err).as_u16(),
        403,
        "TM-WS.3: missing Origin on phone_ws must yield 403"
    );
}

/// TM-WS.3 — When `public_origin` is unset (development / pre-prod),
/// missing Origin MUST NOT be rejected. The upgrade should succeed and
/// the Origin gate must stay disabled — only the production fail-loud
/// check (TM-WS.9) is responsible for catching a misconfigured prod.
#[tokio::test]
async fn phone_ws_allows_missing_origin_when_public_origin_unset() {
    let (port, _key) = spawn_gateway(None).await;
    let token = SessionToken::generate();
    let url = format!("ws://127.0.0.1:{port}/api/phone/{}", token.as_str());

    // Token is well-formed but not registered. The server will accept
    // the upgrade (101), then send an Error frame (no such session) and
    // close — that is fine; what we care about is that we got past the
    // Origin gate, which is signalled by the 101 itself.
    let (ws, response) = tokio_tungstenite::connect_async(ws_request_no_origin(&url))
        .await
        .expect("upgrade must succeed when public_origin is unset");
    assert_eq!(
        response.status().as_u16(),
        101,
        "TM-WS.3 dev path: missing Origin must reach the 101 upgrade"
    );
    drop(ws);
}

/// TM-WS.3 asymmetry — Wrapper WS MUST stay permissive on missing
/// Origin even when `public_origin` is configured. Wrappers are CLI
/// processes (no browser) and never send Origin; demanding it would
/// break every legitimate wrapper connection. Policy is documented in
/// `2026-05-23-sec-4.13-websocket.md` §1.3.
#[tokio::test]
async fn wrapper_ws_allows_missing_origin_even_when_public_origin_configured() {
    let (port, _key) = spawn_gateway(Some(EXPECTED_ORIGIN.to_string())).await;
    let url = format!("ws://127.0.0.1:{port}/api/wrapper");

    let (ws, response) = tokio_tungstenite::connect_async(ws_request_no_origin(&url))
        .await
        .expect("wrapper upgrade must succeed without Origin (CLI-client carveout)");
    assert_eq!(
        response.status().as_u16(),
        101,
        "TM-WS.3 carveout: wrapper must accept missing Origin to keep CLI clients working"
    );
    drop(ws);
}

/// TM-WS.2 regression — Phone WS MUST refuse the upgrade with 403 when
/// `public_origin` is configured and the client sends a *wrong* Origin.
/// This pre-existed the 4.13 fail-closed change; pinning it here keeps a
/// future refactor that consolidates the Origin block from accidentally
/// inverting the equality check.
#[tokio::test]
async fn phone_ws_rejects_wrong_origin() {
    let (port, _key) = spawn_gateway(Some(EXPECTED_ORIGIN.to_string())).await;
    let token = SessionToken::generate();
    let url = format!("ws://127.0.0.1:{port}/api/phone/{}", token.as_str());

    let err =
        tokio_tungstenite::connect_async(ws_request_with_origin(&url, "https://attacker.example"))
            .await
            .expect_err("wrong Origin must be rejected");
    assert_eq!(
        expect_http_status(err).as_u16(),
        403,
        "TM-WS.2: wrong Origin on phone_ws must yield 403"
    );
}
