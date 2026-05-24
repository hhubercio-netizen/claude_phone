// TM-AUTH.7 — structured auth-failure log with correlation ID.
//
// Every auth-failure branch in `wrapper_ws` / `phone_ws` emits a
// `tracing::warn!` with a canonical field set:
//   event="auth_failure", conn_id=<16-hex>, peer_ip=<addr>,
//   reason="<stable taxonomy token>", route=<"wrapper_ws"|"phone_ws">
//
// An operator (or fail2ban) greps one conn_id across the gateway log,
// the reverse-proxy access log, and any downstream pipeline to
// reconstruct an attempt timeline without resorting to noisy
// ip+timestamp heuristics.
//
// These tests are forward-looking. They fail if:
//   - a future refactor drops the `event="auth_failure"` field (breaks
//     log-shipping rules that route on `event`),
//   - a contributor renames a reason ("invalid_api_key" →
//     "bad_api_key") without updating the fail2ban filter & runbook,
//   - a "let's log the rejected key/token for debugging" PR sneaks in
//     and leaks the candidate secret,
//   - the conn_id format changes from 16 lowercase hex (breaks
//     operator greps that have grown to expect the exact shape).
//
// We deliberately do NOT assert on the parent log message text
// ("TM-AUTH.7 auth failure") since that is human-facing and may evolve.
// The structured fields are the contract.

use std::time::Duration;

use claude_phone_gateway::{
    config::{Environment, GatewayConfig, LogFormat},
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
        api_keys: vec![api_key],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
        environment: Environment::Development,
        public_origin: None,
    };
    let app = build_app(&config).unwrap();
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .unwrap();
    tokio::spawn(async move {
        // Mirror the production serve loop so any future axum-layer that
        // changes how the warn! is emitted runs in test too.
        claude_phone_gateway::serve::run(listener, app, std::future::pending::<()>()).await;
    });
    Box::leak(Box::new(static_dir));
    port
}

#[tokio::test]
#[tracing_test::traced_test]
async fn wrapper_invalid_api_key_emits_canonical_auth_failure_log() {
    // The PRIMARY case. The comment at wrapper_ws::handle_socket once
    // claimed "the log line is upstream" — it wasn't. This test pins the
    // log line in place so a future refactor that moves the emission
    // upstream MUST keep the structured shape end-to-end.
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
    let _ = ws.next().await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // All five canonical fields must be present in the captured log
    // output. Match on the structured-field form (key="value") that the
    // default tracing fmt subscriber emits — this is the exact substring
    // a log-shipping rule will grep on, so if the formatter changes the
    // downstream pipeline also breaks and we want that to fail here.
    assert!(
        logs_contain(r#"event="auth_failure""#),
        "TM-AUTH.7: structured field event=\"auth_failure\" missing"
    );
    assert!(
        logs_contain(r#"reason="invalid_api_key""#),
        "TM-AUTH.7: reason taxonomy token missing"
    );
    assert!(
        logs_contain(r#"route="wrapper_ws""#),
        "TM-AUTH.7: route field missing"
    );
    assert!(
        logs_contain("conn_id="),
        "TM-AUTH.7: conn_id correlator missing"
    );
    assert!(logs_contain("peer_ip="), "TM-AUTH.7: peer_ip field missing");

    // The rejected api_key MUST NOT appear anywhere in the log. Even a
    // 4-char prefix would let a brute-forcer correlate attempts and
    // narrow the keyspace.
    assert!(
        !logs_contain(&bad_key_str),
        "TM-AUTH.7: rejected api_key leaked into the log"
    );
}

#[tokio::test]
#[tracing_test::traced_test]
async fn phone_session_not_found_emits_canonical_auth_failure_log() {
    // Second-route pinning: ensures the same shape is emitted by phone_ws
    // and that the `reason` taxonomy includes the post-upgrade
    // session_not_found branch (well-formed token, no matching session).
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // A 43-char base64url token that's syntactically perfect but unknown
    // server-side — exercises the attach_phone Err branch.
    let bad_token = SessionToken::generate();
    let bad_token_str = bad_token.as_str().to_string();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{bad_token_str}"
    ))
    .await
    .unwrap();
    let _ = ws.next().await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    assert!(
        logs_contain(r#"event="auth_failure""#),
        "TM-AUTH.7: event field missing on phone_ws path"
    );
    assert!(
        logs_contain(r#"reason="session_not_found""#),
        "TM-AUTH.7: phone session_not_found taxonomy token missing"
    );
    assert!(
        logs_contain(r#"route="phone_ws""#),
        "TM-AUTH.7: phone_ws route field missing"
    );
    assert!(logs_contain("conn_id="));
    assert!(logs_contain("peer_ip="));

    // The unregistered token must not be persisted to the log — it
    // could be a near-miss of a real session token; logging it would
    // hand it to anyone with log-read access.
    assert!(
        !logs_contain(&bad_token_str),
        "TM-AUTH.7: rejected token leaked into the log"
    );
}

/// Conn_id is 16 lowercase hex chars. Hard-pin the format so a future
/// "let's bump to 32 hex for less collision" refactor or "let's use a
/// uuid for prettiness" PR breaks here — both are fine changes but
/// require coordinated updates to fail2ban grep patterns and the
/// log-correlation runbook, and this test forces that conversation.
#[tokio::test]
#[tracing_test::traced_test]
async fn auth_failure_conn_id_is_16_lowercase_hex_chars() {
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
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
    let _ = ws.next().await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Grep the captured log for `conn_id=` and require the next 16 bytes
    // to be lowercase hex. `logs_assert` runs the closure against the
    // captured lines; returning Err fails the test with the message.
    logs_assert(|lines: &[&str]| {
        let found = lines.iter().any(|l| {
            if let Some(idx) = l.find("conn_id=") {
                let rest = &l[idx + "conn_id=".len()..];
                let id: String = rest.chars().take(16).collect();
                id.len() == 16
                    && id
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            } else {
                false
            }
        });
        if found {
            Ok(())
        } else {
            Err("TM-AUTH.7: no log line contains conn_id=<16 lowercase hex>".into())
        }
    });
}
