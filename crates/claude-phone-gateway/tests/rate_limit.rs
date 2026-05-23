//! Forward-looking integration tests for TM-RATE.1 (per-IP HTTP cap),
//! TM-RATE.2 (auth-failure lockout), TM-RATE.3 (per-connection sliding-
//! window message limiter), and TM-RATE.9 (slow-loris header_read_timeout).
//!
//! All tests drive the exact `serve::run_with` path the binary uses, not
//! axum::serve, so a future refactor that drops the GovernorLayer, the
//! AuthRateLimiter, the ConnRateLimiter wiring, or the header_read_timeout
//! will fail CI here.

use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
    rate_limit::{
        AUTH_FAIL_THRESHOLD, GW_TO_PHONE_MSG_PER_SEC, PER_IP_BURST, PER_IP_REQ_PER_SEC,
        SINK_SEND_TIMEOUT,
    },
    serve,
};
use claude_phone_shared::{
    protocol::{ControlMessage, WrapperHello},
    ApiKey, SessionToken,
};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

/// Spawn a gateway on a free port using the production serve loop.
///
/// `header_read_timeout` is wired through `serve::run_with` so a test
/// can either keep production behaviour (HEADER_READ_TIMEOUT) for cases
/// that don't care about slow-loris, or shrink it to a test budget for
/// the slow-loris test itself. Returns the port the listener is bound to.
async fn spawn_gateway(header_read_timeout: Duration) -> u16 {
    spawn_gateway_with_key(header_read_timeout, ApiKey::generate())
        .await
        .0
}

/// Like `spawn_gateway` but returns the configured `ApiKey` so tests that
/// need to drive a known-good vs known-bad auth flow can reuse the helper.
async fn spawn_gateway_with_key(header_read_timeout: Duration, api_key: ApiKey) -> (u16, ApiKey) {
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
    (port, api_key)
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

/// TM-RATE.2 — `AUTH_FAIL_THRESHOLD` failed WrapperHello attempts from one
/// source IP MUST trigger a per-IP lockout, after which a subsequent
/// upgrade attempt — even with a valid api key — is rejected with HTTP
/// 429 before the WebSocket handshake completes.
///
/// The 250 ms pacing between attempts is deliberate: TM-RATE.1's per-IP
/// HTTP cap admits ~5 r/s (1 token / 200 ms in the governor leaky
/// bucket). Without that pacing we would race the HTTP cap and a 429
/// here could ambiguously be either guard firing — the test must prove
/// it is specifically TM-RATE.2 after `AUTH_FAIL_THRESHOLD` failures.
#[tokio::test]
async fn wrapper_auth_failures_trigger_per_ip_lockout() {
    let api_key = ApiKey::generate();
    let (port, valid_key) = spawn_gateway_with_key(serve::HEADER_READ_TIMEOUT, api_key).await;
    let wrong_key = ApiKey::generate();
    assert_ne!(
        wrong_key.as_str(),
        valid_key.as_str(),
        "test invariant: wrong_key must differ from valid_key"
    );

    let url = format!("ws://127.0.0.1:{port}/api/wrapper");

    for i in 0..AUTH_FAIL_THRESHOLD {
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .unwrap_or_else(|e| panic!("attempt {i}: upgrade should succeed: {e:?}"));
        let hello = ControlMessage::WrapperHello(WrapperHello {
            api_key: wrong_key.clone(),
            token: SessionToken::generate(),
            cols: 80,
            rows: 24,
        });
        ws.send(Message::Text(
            serde_json::to_string(&hello).expect("hello json"),
        ))
        .await
        .ok();
        // drain whatever the server sends in response (typically an
        // Error frame followed by Close) so the socket is cleanly done
        // before the next iteration.
        let _ = tokio::time::timeout(Duration::from_millis(200), async {
            use futures::StreamExt;
            while ws.next().await.is_some() {}
        })
        .await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // The next attempt — using the GOOD key — must still be locked out.
    // If TM-RATE.2 is wired correctly this returns 429 at the HTTP layer
    // before the WS handshake completes; tokio_tungstenite surfaces that
    // as `Error::Http`.
    let err = tokio_tungstenite::connect_async(&url)
        .await
        .expect_err("post-threshold attempt must be rejected");
    let status = match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status(),
        other => panic!("expected Http error, got: {other:?}"),
    };
    assert_eq!(
        status.as_u16(),
        429,
        "lockout must return 429 (TM-RATE.2), got {status:?}"
    );
}

/// RL-I3 — TM-RATE.3. A wrapper that floods more than
/// `GW_TO_PHONE_MSG_PER_SEC` binary frames inside a single second MUST have
/// its session torn down. The per-connection sliding-window limiter lives
/// inside `wrapper_ws::outgoing_task`; if it gets deleted or down-graded,
/// this test fails because the server stops closing the socket under flood.
///
/// We send `cap + 50` frames as fast as `send()` will accept them. The
/// server's outgoing_task processes them on its own pace; the test passes
/// as long as the server closes the socket within a generous wall-clock
/// cap. A future regression that silently drops the limiter would keep
/// the socket open forever, blowing the timeout.
#[tokio::test]
async fn wrapper_message_flood_closes_session() {
    let api_key = ApiKey::generate();
    let (port, valid_key) = spawn_gateway_with_key(serve::HEADER_READ_TIMEOUT, api_key).await;
    let url = format!("ws://127.0.0.1:{port}/api/wrapper");

    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("upgrade should succeed");
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: valid_key.clone(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    });
    ws.send(Message::Text(
        serde_json::to_string(&hello).expect("hello json"),
    ))
    .await
    .expect("send hello");

    // Drain ServerHello so the next read sits on user-data frames only.
    let _server_hello = tokio::time::timeout(Duration::from_millis(500), ws.next())
        .await
        .expect("server_hello within 500ms")
        .expect("server_hello frame present")
        .expect("server_hello not an error");

    // Flood. `cap + 50` is well over the per-connection cap; the smallest
    // payload that's still a valid binary frame keeps the test cheap.
    let burst = GW_TO_PHONE_MSG_PER_SEC + 50;
    for _ in 0..burst {
        // Best-effort: if the peer closes mid-burst, send() errors and we
        // bail — that's actually the success path.
        if ws.send(Message::Binary(vec![0u8; 1])).await.is_err() {
            break;
        }
    }

    // The server MUST close the socket within a generous wall-clock cap.
    // 2 seconds is plenty given the rate cap is 1s sliding window.
    let close_result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await {
                None => return,                        // stream ended = close
                Some(Err(_)) => return,                // peer closed = close
                Some(Ok(Message::Close(_))) => return, // explicit close frame
                Some(Ok(_)) => continue,               // drain any echoed/queued frame
            }
        }
    })
    .await;

    assert!(
        close_result.is_ok(),
        "wrapper flood should have closed the session within 2s; \
         TM-RATE.3 ConnRateLimiter regression suspected"
    );
}

/// RL-I5 — TM-RATE.6 SINK_SEND_TIMEOUT must be a bounded, non-degenerate
/// value. A defender that sets the timeout to hours has effectively no
/// slow-write defense; a defender that sets it sub-second kills honest
/// mobile peers during transient stalls. The constant is also imported by
/// both `wrapper_ws.rs` and `phone_ws.rs`; removing the timeout wrappers
/// without also removing this import would fail clippy `-D warnings` for
/// an unused import. Together with this assertion, that gives a two-sided
/// forward-looking guard against silent weakening of the defense.
#[test]
fn sink_send_timeout_is_bounded_and_reasonable() {
    assert!(
        SINK_SEND_TIMEOUT >= Duration::from_secs(1),
        "TM-RATE.6: sub-second SINK_SEND_TIMEOUT would kill honest mobile peers"
    );
    assert!(
        SINK_SEND_TIMEOUT <= Duration::from_secs(30),
        "TM-RATE.6: SINK_SEND_TIMEOUT > 30 s effectively disables slow-write defense"
    );
}
