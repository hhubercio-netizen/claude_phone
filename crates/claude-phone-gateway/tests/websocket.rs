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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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

/// Parsed HTTP response of a raw-TCP WebSocket upgrade. The fields hold
/// only what the TM-WS.8 / TM-WS.12 assertions need: the status line and
/// a lowercase-keyed map of headers. Used to bypass tungstenite's strict
/// client-side validation of `Sec-WebSocket-Protocol` and let the test
/// inspect what the server actually sent.
struct RawUpgradeResponse {
    status: u16,
    headers: Vec<(String, String)>,
}

impl RawUpgradeResponse {
    fn header(&self, name: &str) -> Option<&str> {
        let lname = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&lname))
            .map(|(_, v)| v.as_str())
    }
}

/// Drive a raw WebSocket upgrade over plain TCP and return the parsed
/// 101 response. tungstenite 0.24 fails the client-side handshake when
/// the client offered `Sec-WebSocket-Protocol` but the server didn't
/// echo one back (RFC 6455 §4.1 "MUST" wording is interpreted strictly).
/// Our server intentionally never echoes one (TM-WS.12), so the only
/// way to assert on the response headers is to bypass the strict client.
///
/// The Sec-WebSocket-Key value below is the RFC 6455 §1.2 sample nonce;
/// the server doesn't validate its entropy, only that it parses as a
/// 16-byte base64 string and that the resulting Sec-WebSocket-Accept is
/// computed correctly on its side.
async fn raw_ws_upgrade(
    port: u16,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> RawUpgradeResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("tcp connect");

    let mut req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n"
    );
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await.expect("write req");

    // Read until end-of-headers marker. WS 101 has an empty body so
    // \r\n\r\n is also the end of the parseable portion. Cap the read
    // window at 32 KiB — way more than the production response which
    // is well under 2 KiB even with all security headers.
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while buf.windows(4).all(|w| w != b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let n = tokio::time::timeout(remaining, stream.read(&mut chunk))
            .await
            .expect("response within 2s")
            .expect("read");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 32 * 1024 {
            panic!("response headers exceeded 32 KiB before terminator");
        }
    }

    // The phone WS route sends a binary Close frame immediately after the
    // 101 (no matching session for the synthetic token), so the bytes
    // *after* \r\n\r\n are not valid UTF-8. Slice the header window only.
    let end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("end-of-headers marker present");
    let header_window = &buf[..end];
    let text = std::str::from_utf8(header_window).expect("response headers are utf-8");
    let mut lines = text.split("\r\n");
    let status_line = lines.next().expect("status line");
    let mut parts = status_line.split(' ');
    let _http_version = parts.next().expect("http version");
    let status: u16 = parts
        .next()
        .expect("status code")
        .parse()
        .expect("status u16");

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    RawUpgradeResponse { status, headers }
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

// --- TM-WS.8 — permessage-deflate compression must never be negotiated ----

/// TM-WS.8 — Phone WS MUST NOT advertise `permessage-deflate` in its
/// 101 response even when the client offers it. Compression over tiny
/// PTY frames + attacker-controlled content is the classic CRIME/BREACH
/// oracle shape; we never want it on. The default axum behaviour is
/// "don't negotiate", and this test pins that default so a future
/// `WebSocketUpgrade::with_compression(true)`-style call would flip CI red.
#[tokio::test]
async fn compression_extension_not_negotiated_phone() {
    let (port, _key) = spawn_gateway(None).await;
    let token = SessionToken::generate();
    let path = format!("/api/phone/{}", token.as_str());

    let response = raw_ws_upgrade(
        port,
        &path,
        &[("Sec-WebSocket-Extensions", "permessage-deflate")],
    )
    .await;
    assert_eq!(response.status, 101, "expected 101 upgrade");
    let extensions = response
        .header("sec-websocket-extensions")
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(
        !extensions.contains("permessage-deflate"),
        "TM-WS.8: server MUST NOT negotiate permessage-deflate; got: {extensions:?}"
    );
}

/// TM-WS.8 — same check on the wrapper route. Asymmetric guards table at
/// 4.13 sub-spec §1.3 keeps both routes symmetric on compression: both
/// must refuse, both have this test.
#[tokio::test]
async fn compression_extension_not_negotiated_wrapper() {
    let (port, _key) = spawn_gateway(None).await;

    let response = raw_ws_upgrade(
        port,
        "/api/wrapper",
        &[("Sec-WebSocket-Extensions", "permessage-deflate")],
    )
    .await;
    assert_eq!(response.status, 101, "expected 101 upgrade");
    let extensions = response
        .header("sec-websocket-extensions")
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(
        !extensions.contains("permessage-deflate"),
        "TM-WS.8: wrapper server MUST NOT negotiate permessage-deflate; got: {extensions:?}"
    );
}

// --- TM-WS.12 — Sec-WebSocket-Protocol must never be negotiated -----------

/// TM-WS.12 — Phone WS MUST NOT echo a `Sec-WebSocket-Protocol` header
/// in its 101 response, regardless of what the client offered. We do not
/// version the protocol via subprotocols today; any future change that
/// does must introduce explicit strict-match negotiation, audited
/// separately. This test catches a future contributor calling
/// `WebSocketUpgrade::protocols(...)` and silently negotiating the
/// first client-offered value.
///
/// Driven via raw TCP because tungstenite 0.24 fails the client-side
/// handshake when the client offered a subprotocol but the server
/// didn't echo one — that is exactly the path we want to assert on, so
/// we bypass the strict client and read the 101 headers ourselves.
#[tokio::test]
async fn subprotocol_not_negotiated_phone() {
    let (port, _key) = spawn_gateway(None).await;
    let token = SessionToken::generate();
    let path = format!("/api/phone/{}", token.as_str());

    let response = raw_ws_upgrade(
        port,
        &path,
        &[("Sec-WebSocket-Protocol", "claude-phone-v0, chat-v1")],
    )
    .await;
    assert_eq!(response.status, 101, "expected 101 upgrade");
    assert!(
        response.header("sec-websocket-protocol").is_none(),
        "TM-WS.12: server MUST NOT select a subprotocol; got: {:?}",
        response.header("sec-websocket-protocol")
    );
}

/// TM-WS.12 — same check on the wrapper route. Both routes share the
/// "no subprotocol negotiation" baseline per §1.3 asymmetric-guard table.
#[tokio::test]
async fn subprotocol_not_negotiated_wrapper() {
    let (port, _key) = spawn_gateway(None).await;

    let response = raw_ws_upgrade(
        port,
        "/api/wrapper",
        &[("Sec-WebSocket-Protocol", "claude-phone-v0, chat-v1")],
    )
    .await;
    assert_eq!(response.status, 101, "expected 101 upgrade");
    assert!(
        response.header("sec-websocket-protocol").is_none(),
        "TM-WS.12: wrapper server MUST NOT select a subprotocol; got: {:?}",
        response.header("sec-websocket-protocol")
    );
}

// --- TM-WS.7 / TM-WS.10 — asymmetry pins (4.6 implements; 4.13 verifies) --

/// TM-WS.10 — Both routes MUST close the socket inside `HEADER_READ_TIMEOUT`
/// when the client never finishes the request headers (classic slow-loris).
/// 4.6 wired `header_read_timeout(Some(10 s))` into the hyper builder; this
/// test asserts the timeout fires for BOTH URL paths within the same window,
/// so a future refactor that mounts one route on a different server / builder
/// can't quietly desync the timeout.
#[tokio::test]
async fn both_routes_close_on_slow_loris_headers() {
    let (port, _key) = spawn_gateway(None).await;
    let token = SessionToken::generate();
    let phone_path = format!("/api/phone/{}", token.as_str());

    let routes: [(&str, &str); 2] = [("/api/wrapper", "wrapper"), (phone_path.as_str(), "phone")];

    let mut elapsed = Vec::new();
    for (path, name) in routes {
        let start = std::time::Instant::now();
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("tcp connect");
        // Partial request: request line + one header, NO terminating
        // \r\n\r\n. The server must hit HEADER_READ_TIMEOUT and close.
        let prefix = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n"
        );
        stream
            .write_all(prefix.as_bytes())
            .await
            .expect("write partial headers");

        // Drain until the server half-closes. Wrap in a hard ceiling so
        // a regression that drops the timeout doesn't hang CI for hours.
        let mut sink = [0u8; 1024];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut sink))
                .await
                .unwrap_or_else(|_| panic!("TM-WS.10 ({name}): no close within 30 s"))
                .expect("read");
            if n == 0 {
                break;
            }
        }
        elapsed.push((name, start.elapsed()));
    }

    let (wrapper_name, wrapper_t) = elapsed[0];
    let (phone_name, phone_t) = elapsed[1];

    // HEADER_READ_TIMEOUT = 10 s; allow +5 s for hyper scheduling + Windows
    // TCP teardown jitter. Floor at 5 s so a regression that drops the
    // timeout entirely (close-on-FIN immediately) also trips the assert.
    let upper = Duration::from_secs(15);
    let lower = Duration::from_secs(5);
    assert!(
        wrapper_t <= upper,
        "TM-WS.10 ({wrapper_name}): close at {wrapper_t:?} > {upper:?}"
    );
    assert!(
        phone_t <= upper,
        "TM-WS.10 ({phone_name}): close at {phone_t:?} > {upper:?}"
    );
    assert!(
        wrapper_t >= lower,
        "TM-WS.10 ({wrapper_name}): close at {wrapper_t:?} < {lower:?} — premature close means the timeout isn't actually waiting on the headers"
    );
    assert!(
        phone_t >= lower,
        "TM-WS.10 ({phone_name}): close at {phone_t:?} < {lower:?} — premature close"
    );

    let diff = wrapper_t.abs_diff(phone_t);
    assert!(
        diff <= Duration::from_secs(3),
        "TM-WS.10 asymmetry: wrapper={wrapper_t:?} phone={phone_t:?} diff={diff:?} > 3 s — routes drifted"
    );
}

// --- TM-WS.7 helpers ------------------------------------------------------
//
// The pong-deadline asymmetry pin needs to drive both routes past their
// post-upgrade gates (wrapper HELLO_TIMEOUT, phone token-resolution) and
// then go silent. tokio-tungstenite would auto-Pong, so we hand-write the
// HELLO frame on a raw socket. RFC 6455 §5.2 frame format, client-to-server
// MUST be masked.

async fn raw_ws_complete_upgrade(port: u16, path: &str) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("tcp connect");
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .expect("write handshake");
    let mut chunk = [0u8; 1024];
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    while buf.windows(4).all(|w| w != b"\r\n\r\n") {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("upgrade response within 5 s")
            .expect("read");
        if n == 0 {
            panic!("peer closed before upgrade completed");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    stream
}

async fn ws_send_masked_text(stream: &mut TcpStream, payload: &str) {
    let bytes = payload.as_bytes();
    let len = bytes.len();
    let mut frame: Vec<u8> = Vec::with_capacity(14 + len);
    frame.push(0x81); // FIN=1, opcode=text
    if len < 126 {
        frame.push(0x80 | (len as u8));
    } else if len <= 65_535 {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    // Mask key. Entropy doesn't matter for unmasking; pin a constant so
    // the wire is reproducible if a future debugger ever taps it.
    let mask = [0xa5_u8, 0x5a, 0x3c, 0xc3];
    frame.extend_from_slice(&mask);
    for (i, b) in bytes.iter().enumerate() {
        frame.push(b ^ mask[i & 3]);
    }
    stream.write_all(&frame).await.expect("write text frame");
}

/// TM-WS.7 — Both routes MUST drop a peer that stops responding to server
/// Pings within `PONG_DEADLINE` (4.6 lands a 90 s deadline, ping every
/// 30 s). 4.6's `rate_limit.rs` test pins the constant; this asymmetry
/// pin asserts the *runtime* behaviour matches on BOTH URL paths, so a
/// future copy-pasted refactor that breaks the watchdog on one side trips
/// CI even when the constant looks fine.
///
/// Driven via raw TCP because tokio-tungstenite's `read` loop transparently
/// answers server Pings with Pongs, which would defeat the watchdog under
/// test. The raw socket sends a single HELLO text frame (to satisfy the
/// post-upgrade gate so the keepalive task starts) and then drains
/// everything else without ever sending a Pong.
///
/// Marked `#[ignore]` because PONG_DEADLINE = 90 s makes this a >2 minute
/// test. Run explicitly with:
///
/// ```text
/// cargo test -p claude-phone-gateway --test websocket -- --ignored both_routes_drop_on_no_pong_within_deadline
/// ```
#[tokio::test]
#[ignore = "slow ~125 s — TM-WS.7 asymmetry pin, run nightly or on-demand"]
async fn both_routes_drop_on_no_pong_within_deadline() {
    let (port, api_key) = spawn_gateway(None).await;
    let token = SessionToken::generate();

    // Wrapper: establishes the session by sending its WrapperHello first.
    let mut wrapper = raw_ws_complete_upgrade(port, "/api/wrapper").await;
    let wrapper_hello = format!(
        r#"{{"type":"wrapper_hello","api_key":"{}","token":"{}","cols":80,"rows":24}}"#,
        api_key.as_str(),
        token.as_str()
    );
    ws_send_masked_text(&mut wrapper, &wrapper_hello).await;

    // Phone: connects to the now-registered token and identifies itself.
    let mut phone = raw_ws_complete_upgrade(port, &format!("/api/phone/{}", token.as_str())).await;
    let phone_hello = format!(
        r#"{{"type":"phone_hello","token":"{}","cols":80,"rows":24}}"#,
        token.as_str()
    );
    ws_send_masked_text(&mut phone, &phone_hello).await;

    // Start the clock now — both keepalive loops have been kicked off by
    // their HELLO. The 90 s PONG_DEADLINE runs from socket_start, which is
    // captured at task spawn (right after HELLO is accepted).
    let started = std::time::Instant::now();

    // Drain both in parallel without ever sending a Pong. A 180 s read
    // timeout caps the wait so a regression that disables the watchdog
    // fails the test instead of hanging the runner indefinitely.
    let drain = |mut s: TcpStream, name: &'static str| async move {
        let started = std::time::Instant::now();
        let mut sink = [0u8; 4096];
        loop {
            match tokio::time::timeout(Duration::from_secs(180), s.read(&mut sink)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => panic!("TM-WS.7 ({name}): read error: {e}"),
                Err(_) => panic!(
                    "TM-WS.7 ({name}): no close after 180 s of no-pong — watchdog not firing"
                ),
            }
        }
        started.elapsed()
    };
    let wrapper_handle = tokio::spawn(drain(wrapper, "wrapper"));
    let phone_handle = tokio::spawn(drain(phone, "phone"));
    let wrapper_t = wrapper_handle.await.expect("wrapper drain task");
    let phone_t = phone_handle.await.expect("phone drain task");

    // Sanity: the test itself should be done in well under the per-stream
    // timeout. If we see >150 s here something is off with the scheduler.
    assert!(
        started.elapsed() <= Duration::from_secs(150),
        "test wall-clock exceeded 150 s, asymmetry pin disrupted"
    );

    // Window per route: PONG_DEADLINE (90 s) + one ping_interval (30 s) + 5 s
    // jitter. Floor: PONG_DEADLINE - one ping_interval, since the first
    // Ping fires anywhere within `[0, 30 s)` of socket_start.
    let upper = Duration::from_secs(125);
    let lower = Duration::from_secs(60);
    assert!(
        wrapper_t <= upper,
        "TM-WS.7 (wrapper): close at {wrapper_t:?} > {upper:?}"
    );
    assert!(
        phone_t <= upper,
        "TM-WS.7 (phone): close at {phone_t:?} > {upper:?}"
    );
    assert!(
        wrapper_t >= lower,
        "TM-WS.7 (wrapper): close at {wrapper_t:?} < {lower:?} — deadline shrank"
    );
    assert!(
        phone_t >= lower,
        "TM-WS.7 (phone): close at {phone_t:?} < {lower:?} — deadline shrank"
    );

    let diff = wrapper_t.abs_diff(phone_t);
    assert!(
        diff <= Duration::from_secs(35),
        "TM-WS.7 asymmetry: wrapper={wrapper_t:?} phone={phone_t:?} diff={diff:?} > 35 s — routes drifted"
    );
}
