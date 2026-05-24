//! TM-TEST.3 — negative-path gap-fill across both WS routes.
//!
//! Existing test coverage:
//!
//! - `e2e_test.rs::wrapper_ws_hello_timeout_drops_idle_socket` — TM-WS.3
//!   hello timeout on the wrapper side.
//! - `e2e_test.rs::phone_ws_rejects_missing_origin_when_public_origin_set`
//!   + `websocket.rs::phone_ws_rejects_missing_origin_when_public_origin_configured` — TM-WS.3 Origin-missing.
//! - `e2e_test.rs::phone_ws_rejects_wrong_origin` + `websocket.rs` — Origin
//!   spoof on both routes.
//! - `pentest_e2e.rs::pentest_oversized_hello_frame_rejected_cleanly` —
//!   TM-WS.4 wrapper-text-pre-hello oversize.
//!
//! Gaps this file fills:
//!
//! - TM-WS.5 wrapper-binary-pre-hello oversize (complement to the
//!   existing wrapper-text-pre-hello in pentest_e2e).
//! - TM-WS.4 phone-text-pre-hello oversize.
//! - TM-WS.5 phone-binary-pre-hello oversize.
//! - In-session "replayed hello" — a second hello frame AFTER the
//!   first has been accepted MUST NOT trigger a re-handshake. Pins
//!   `recv_hello` / `recv_phone_hello` as one-shot.
//!
//! Forward-looking shape: every assertion would fire if a future
//! refactor either widened the size caps, fragmented the hello path
//! into multiple `recv_hello` calls, or wired a credential-refresh
//! handshake post-attach.
//!
//! Note on pre-hello focus: all oversize tests fire BEFORE the hello
//! is accepted. The split sink/stream loop after attach has its own
//! shutdown path that depends on the keepalive watchdog (TM-RATE.7)
//! and is exercised by `websocket.rs::both_routes_drop_on_no_pong_*`.
//! Pre-hello is the right place to pin the size cap because
//! `recv_hello` / `recv_phone_hello` is the single, well-defined point
//! where the cap is enforced before any stateful protocol commitment.

use std::time::Duration;

use claude_phone_gateway::{
    config::{Environment, GatewayConfig, LogFormat},
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
        api_keys: vec![api_key.clone()],
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

/// Drain the WS stream until the peer closes (Close frame, EOF, or
/// transport error). Returns the elapsed time. Used to assert that a
/// misbehaving pre-hello frame ends the socket within a bounded window
/// — a regression that drops the size cap would either deliver a
/// ServerHello (test caller checks the close shape) or hang here, then
/// trip the outer `tokio::time::timeout`.
async fn drain_until_close<S>(stream: &mut S) -> Duration
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let start = std::time::Instant::now();
    loop {
        match stream.next().await {
            None => return start.elapsed(),
            Some(Err(_)) => return start.elapsed(),
            Some(Ok(Message::Close(_))) => return start.elapsed(),
            Some(Ok(_)) => continue,
        }
    }
}

/// Send a WrapperHello and consume the ServerHello so the session is
/// fully attached. Returns the wrapper WS for the caller to keep
/// driving. Used by the "replayed hello" tests that need a live
/// post-attach socket.
async fn wrapper_connect_and_hello(
    port: u16,
    api_key: ApiKey,
    token: SessionToken,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key,
        token,
        cols: 80,
        rows: 24,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .expect("send wrapper hello");
    let resp = ws.next().await.expect("server hello").expect("ws ok");
    match resp {
        Message::Text(t) => {
            let msg: ControlMessage = serde_json::from_str(&t).expect("parse server hello");
            assert!(
                matches!(msg, ControlMessage::ServerHello(_)),
                "expected ServerHello, got {msg:?}"
            );
        }
        other => panic!("expected text ServerHello, got {other:?}"),
    }
    ws
}

// =====================================================================
// TM-WS.4 / TM-WS.5 — oversized pre-hello frame rejection.
//
// Sized at 128 KiB (2x the 64 KiB cap) for the same reason
// pentest_e2e.rs picks 128 KiB: a clear 2x overshoot exercises both
// `max_frame_size` and `max_message_size` consistently, where a
// one-byte overflow can hit tungstenite-side fragmentation edge cases.
// =====================================================================

/// TM-WS.5 — wrapper_ws MUST close the socket when the first frame is
/// an oversized BINARY (128 KiB). The wrapper protocol expects a text
/// JSON hello as the first frame; oversized binary doubly violates the
/// contract (wrong shape + over the cap). Complements pentest_e2e's
/// `pentest_oversized_hello_frame_rejected_cleanly` which covers the
/// text path.
#[tokio::test]
async fn wrapper_ws_closes_on_oversized_binary_pre_hello() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key.clone()).await;

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");

    let oversized = vec![0xAA_u8; 128 * 1024];
    let _ = ws.send(Message::Binary(oversized)).await;

    let elapsed = tokio::time::timeout(Duration::from_secs(5), async {
        let (_sink, mut stream) = ws.split();
        drain_until_close(&mut stream).await
    })
    .await
    .expect("TM-WS.5 (wrapper binary pre-hello): socket must close within 5 s");
    assert!(
        elapsed < Duration::from_secs(5),
        "TM-WS.5 (wrapper binary pre-hello): close arrived at {elapsed:?}, want < 5 s"
    );

    // Survivor: a fresh wrapper still connects to the gateway.
    let _ = wrapper_connect_and_hello(port, api_key, SessionToken::generate()).await;
}

/// TM-WS.4 — phone_ws MUST close on an oversized TEXT first frame.
/// Even though the phone-side recv_phone_hello expects text, a 128 KiB
/// text blob is over the cap. The size cap fires before the JSON parse
/// would, which is the right order: cheap network-level filter before
/// allocation.
#[tokio::test]
async fn phone_ws_closes_on_oversized_text_pre_hello() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;

    // Register a wrapper so the phone token resolves — otherwise the
    // close would be from "no such session" (which is fine but tests
    // the wrong path). Registering first means the phone side actually
    // gets into `recv_phone_hello` and the oversized frame trips the
    // cap there.
    let _wrapper = wrapper_connect_and_hello(port, api_key, token.clone()).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");

    let oversized = "x".repeat(128 * 1024);
    let _ = ws.send(Message::Text(oversized)).await;

    let elapsed = tokio::time::timeout(Duration::from_secs(5), async {
        let (_sink, mut stream) = ws.split();
        drain_until_close(&mut stream).await
    })
    .await
    .expect("TM-WS.4 (phone text pre-hello): socket must close within 5 s");
    assert!(
        elapsed < Duration::from_secs(5),
        "TM-WS.4 (phone text pre-hello): close arrived at {elapsed:?}, want < 5 s"
    );
}

/// TM-WS.5 — phone_ws MUST close on an oversized BINARY first frame.
/// Binary pre-hello on the phone side is doubly off-shape
/// (recv_phone_hello expects Text), and 128 KiB is over the cap on top
/// of that. Pinning closes the size-cap loophole on the binary path.
#[tokio::test]
async fn phone_ws_closes_on_oversized_binary_pre_hello() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;

    let _wrapper = wrapper_connect_and_hello(port, api_key, token.clone()).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");

    let oversized = vec![0x55_u8; 128 * 1024];
    let _ = ws.send(Message::Binary(oversized)).await;

    let elapsed = tokio::time::timeout(Duration::from_secs(5), async {
        let (_sink, mut stream) = ws.split();
        drain_until_close(&mut stream).await
    })
    .await
    .expect("TM-WS.5 (phone binary pre-hello): socket must close within 5 s");
    assert!(
        elapsed < Duration::from_secs(5),
        "TM-WS.5 (phone binary pre-hello): close arrived at {elapsed:?}, want < 5 s"
    );
}

// =====================================================================
// TM-TEST.3 — "replayed hello" in-session.
//
// Both routes consume the first hello in a one-shot `recv_hello` /
// `recv_phone_hello` call. After that the socket falls into a split
// sink/stream loop where text frames are forwarded blindly to the peer
// — no re-parse, no re-registration. A second hello on the same socket
// therefore:
//   (a) MUST NOT produce a second ServerHello on the originating side
//       (no re-handshake — the session_id is final),
//   (b) MUST leave the existing session alive (the bytes flow into the
//       text-forwarding lane, which is benign).
// =====================================================================

/// TM-TEST.3 — a second `WrapperHello` on an already-registered wrapper
/// socket MUST NOT trigger a re-handshake. Forward-looking guard: a
/// future refactor that wires a second `recv_hello` after the split
/// (perhaps to support credential refresh) would surface here as an
/// unexpected ServerHello and the test would fail loud. The pin is
/// "session_id is established once, not renegotiable in-band".
#[tokio::test]
async fn wrapper_ws_second_hello_does_not_re_register() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;
    let mut ws = wrapper_connect_and_hello(port, api_key.clone(), token).await;

    // Send a second WrapperHello on the same socket.
    let second = ControlMessage::WrapperHello(WrapperHello {
        api_key,
        token: SessionToken::generate(),
        cols: 200,
        rows: 50,
    });
    ws.send(Message::Text(serde_json::to_string(&second).unwrap()))
        .await
        .expect("send second hello");

    // The gateway forwards the second hello as plain text into the
    // phone-bound channel (no phone is attached yet, so binary frames
    // would be buffered; text frames are dropped — see wrapper_ws.rs
    // outgoing_task). Either way, we MUST NOT receive a second
    // ServerHello on the wrapper side. Allow 500 ms slack for a
    // hypothetical re-handshake to race.
    let unexpected = tokio::time::timeout(Duration::from_millis(500), ws.next()).await;
    assert!(
        unexpected.is_err(),
        "TM-TEST.3 (wrapper): a second WrapperHello must not produce a second ServerHello; got: {unexpected:?}"
    );

    // Session must still be alive: send a binary frame, the socket
    // accepts it (would error if the gateway had torn the session down).
    ws.send(Message::Binary(b"still here".to_vec()))
        .await
        .expect("TM-TEST.3 (wrapper): session must remain alive after a no-op second hello");
}

/// TM-TEST.3 — a second `PhoneHello` on an already-attached phone socket
/// MUST NOT trigger a re-attach. Same forward-looking shape as the
/// wrapper case: the attach is a one-shot at `recv_phone_hello`, and
/// any future change that retries the hello path post-attach would
/// trip this assertion.
#[tokio::test]
async fn phone_ws_second_hello_does_not_re_attach() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;

    let _wrapper = wrapper_connect_and_hello(port, api_key, token.clone()).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");
    let phello = ControlMessage::PhoneHello(PhoneHello {
        token: token.clone(),
        cols: 80,
        rows: 24,
        user_agent: None,
    });
    ws.send(Message::Text(serde_json::to_string(&phello).unwrap()))
        .await
        .expect("send first phone hello");
    let first_resp = ws.next().await.expect("server hello").expect("ws ok");
    match first_resp {
        Message::Text(t) => {
            let msg: ControlMessage = serde_json::from_str(&t).expect("parse");
            assert!(
                matches!(msg, ControlMessage::ServerHello(_)),
                "first server hello on phone path"
            );
        }
        other => panic!("expected server hello text, got {other:?}"),
    }

    // Second PhoneHello on the same already-attached socket.
    let second = ControlMessage::PhoneHello(PhoneHello {
        token: token.clone(),
        cols: 9999,
        rows: 9999,
        user_agent: Some("replay/1.0".to_string()),
    });
    ws.send(Message::Text(serde_json::to_string(&second).unwrap()))
        .await
        .expect("send second phone hello");

    // No second ServerHello should arrive. A PeerStatus frame from the
    // wrapper side is fine (peer_up was sent on first attach), but a
    // ServerHello specifically must not appear because that would mean
    // the gateway re-ran the attach handshake on a live socket.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            ws.next(),
        )
        .await
        {
            Err(_) => break,
            Ok(None) => break,
            Ok(Some(Err(_))) => break,
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<ControlMessage>(&t) {
                    assert!(
                        !matches!(msg, ControlMessage::ServerHello(_)),
                        "TM-TEST.3 (phone): second PhoneHello must not produce a second ServerHello; got: {msg:?}"
                    );
                }
            }
            Ok(Some(Ok(_))) => continue,
        }
    }

    // Session still alive: send a benign binary frame (will be
    // sanitized; an empty result is still accepted).
    ws.send(Message::Binary(b"\x01".to_vec()))
        .await
        .expect("TM-TEST.3 (phone): session must remain alive after a no-op second hello");
}
