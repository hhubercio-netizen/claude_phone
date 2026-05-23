//! Forward-looking integration tests for TM-RATE.1 (per-IP HTTP cap) and
//! TM-RATE.9 (slow-loris header_read_timeout).
//!
//! Both tests drive the exact `serve::run_with` path the binary uses, not
//! axum::serve, so a future refactor that drops the GovernorLayer or the
//! header_read_timeout will fail CI here.

use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
    rate_limit::{PER_IP_BURST, PER_IP_REQ_PER_SEC},
    serve,
};
use claude_phone_shared::ApiKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Spawn a gateway on a free port using the production serve loop.
///
/// `header_read_timeout` is wired through `serve::run_with` so a test
/// can either keep production behaviour (HEADER_READ_TIMEOUT) for cases
/// that don't care about slow-loris, or shrink it to a test budget for
/// the slow-loris test itself. Returns the port the listener is bound to.
async fn spawn_gateway(header_read_timeout: Duration) -> u16 {
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
        public_origin: None,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        // never-completing shutdown — the test process exits which kills
        // the task. A oneshot here would be cleaner but adds nothing the
        // OS doesn't already do at test teardown.
        serve::run_with(
            listener,
            app,
            std::future::pending::<()>(),
            header_read_timeout,
            Duration::from_secs(1),
        )
        .await;
    });
    // tempdir must outlive the spawned task or ServeDir will 404.
    Box::leak(Box::new(static_dir));

    // small settle window so the listener has accepted before clients hit
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

/// RL-I1 — TM-RATE.1. A single client (one source IP) firing well above
/// `PER_IP_REQ_PER_SEC + PER_IP_BURST` requests in tight succession MUST
/// see at least one 429. The test computes the burst from the public
/// constants so a future policy change that lowers the cap does not
/// silently break this test — it stays in lockstep with the symbol.
#[tokio::test]
async fn per_ip_governor_returns_429_under_burst() {
    let port = spawn_gateway(serve::HEADER_READ_TIMEOUT).await;
    let client = reqwest::Client::builder()
        // disable pooling to make sure each request is treated as a fresh
        // request by the governor — pooled keep-alive would reuse the
        // same connection but still count as separate HTTP requests, so
        // either path works; we just want predictability.
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client");

    // Send burst + sustained-for-1s + safety margin. With burst=10 and
    // 5 r/s sustained, sending 30 in a tight loop must produce at least
    // one 429 unless the limiter is bypassed.
    let total: u32 = PER_IP_BURST + (PER_IP_REQ_PER_SEC as u32) + 15;
    let mut statuses = Vec::with_capacity(total as usize);
    for _ in 0..total {
        let resp = client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .send()
            .await
            .expect("request");
        statuses.push(resp.status().as_u16());
    }

    let throttled = statuses.iter().filter(|s| **s == 429).count();
    assert!(
        throttled >= 1,
        "expected at least one 429 in {total} requests, got statuses: {statuses:?}"
    );
}

/// RL-I4 — TM-RATE.9. A client that opens a TCP connection and stalls
/// before completing the HTTP request line / headers MUST have the
/// server tear the connection down within roughly `header_read_timeout`.
/// We use a 300 ms test budget so the test stays under a second; the
/// production constant is 10 s. The test seam `serve::run_with` is what
/// makes this possible without sleeping for the full production window.
#[tokio::test]
async fn slow_loris_header_read_timeout() {
    let test_timeout = Duration::from_millis(300);
    let port = spawn_gateway(test_timeout).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("tcp connect");
    // Send a partial request line and stop. No CRLF, no Host header,
    // nothing that lets hyper parse a complete request. A vulnerable
    // server would keep this socket open indefinitely.
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\n")
        .await
        .expect("partial write");
    stream.flush().await.ok();

    // Read until the peer closes (EOF) or until we hit a generous wall
    // clock that's still well below what a vulnerable server would
    // exhibit. Use 5 * test_timeout as the cap so CI jitter doesn't
    // flake — the guarantee under test is "closes in finite time",
    // not "closes at exactly 300 ms".
    let cap = test_timeout * 5;
    let result = tokio::time::timeout(cap, async {
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) => return Ok::<(), std::io::Error>(()),
                Ok(_) => continue, // hyper may send a 408 first, then close
                Err(e) => return Err(e),
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "slow-loris connection was not closed within {cap:?}; \
         header_read_timeout regression suspected"
    );
}
