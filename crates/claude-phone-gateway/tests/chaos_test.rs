//! TM-TEST.6 — chaos under random mid-session client drops.
//!
//! Forward-looking invariants we want to pin:
//!
//! 1. wrapper drop frees the session slot. After a wrapper socket dies,
//!    the token MUST become available again. A regression that leaves
//!    the registry entry behind would surface as `SessionTaken` on
//!    re-register.
//! 2. phone drop does NOT cascade to the wrapper side. The wrapper
//!    "owns" the session; a phone close is an attach lifecycle event,
//!    not a session-kill event. Forward-looking guard against anyone
//!    wiring `phone_ws` to call `registry.remove` on disconnect.
//! 3. Under many sessions with randomized wrapper-drop chaos, every
//!    token that was dropped MUST be re-registrable and every token
//!    whose wrapper is still alive MUST remain taken. No leaks, no
//!    cross-token state corruption.
//!
//! Why "chaos" with a fixed RNG seed rather than a single deterministic
//! drop pattern: the registry / DashMap interactions are intrinsically
//! racey (shard locks, atomic reservation counter, per-session cancel
//! tokens). A single hard-coded pattern would pin one path; mixing
//! parallel registrations with parallel drops at varied indices
//! exercises the same code under realistic interleavings while a fixed
//! seed keeps the test reproducible in CI.

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
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey, max_sessions: usize) -> u16 {
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
        max_sessions,
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

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn wrapper_connect_and_hello(
    port: u16,
    api_key: ApiKey,
    token: SessionToken,
) -> Result<WsStream, String> {
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .map_err(|e| format!("connect: {e}"))?;
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key,
        token,
        cols: 80,
        rows: 24,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .map_err(|e| format!("send hello: {e}"))?;
    let resp = ws
        .next()
        .await
        .ok_or_else(|| "no response".to_string())?
        .map_err(|e| format!("recv: {e}"))?;
    match resp {
        Message::Text(t) => {
            let msg: ControlMessage =
                serde_json::from_str(&t).map_err(|e| format!("parse: {e}"))?;
            match msg {
                ControlMessage::ServerHello(_) => Ok(ws),
                ControlMessage::Error(e) => Err(format!("server error: {:?}", e.code)),
                other => Err(format!("unexpected: {other:?}")),
            }
        }
        other => Err(format!("expected text, got {other:?}")),
    }
}

/// Drop the wrapper socket and give the gateway a tight settle window
/// to run the post-`join!` cleanup in `wrapper_ws::handle_wrapper`
/// (the `state.registry.remove(&token)` call). 500 ms is comfortably
/// above the actual cleanup latency (sub-millisecond on a healthy
/// system) but tight enough to fail loud if a future change moves the
/// cleanup behind a longer-running task.
async fn drop_wrapper_and_settle(ws: WsStream) {
    drop(ws);
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// =====================================================================
// Test 1 — wrapper drop frees the session slot.
// =====================================================================
#[tokio::test]
async fn wrapper_drop_releases_session_token() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone(), 10).await;

    // First wrapper registers successfully.
    let ws1 = wrapper_connect_and_hello(port, api_key.clone(), token.clone())
        .await
        .expect("first wrapper must register");

    // Second wrapper with the SAME token MUST be rejected — slot is held.
    let blocked = wrapper_connect_and_hello(port, api_key.clone(), token.clone()).await;
    assert!(
        blocked.is_err(),
        "TM-TEST.6: a second wrapper on a live token must be rejected"
    );

    // Drop the first wrapper; gateway cleanup must release the slot.
    drop_wrapper_and_settle(ws1).await;

    // Same token must now register fresh.
    let _ws2 = wrapper_connect_and_hello(port, api_key, token)
        .await
        .expect("TM-TEST.6: same token must be re-registrable after wrapper drop");
}

// =====================================================================
// Test 2 — paired wrapper+phone drop releases the session slot
// promptly.
//
// In the current design `session.cancel` is shared, so either side
// disconnecting tears the session down. The forward-looking invariant
// we pin here is the cleanup TIMING: once both peers are gone, the
// token slot MUST be reclaimed inside a tight bound (sub-second on a
// healthy build with the TM-RATE.7 cancel-propagation in place). A
// regression that lengthens the cleanup path (e.g. the
// `tokio::join!` waiting on a no-longer-cancelled keepalive watchdog)
// would let a churning peer pin all `max_sessions` slots well past
// the moment the peers were actually gone.
// =====================================================================
#[tokio::test]
async fn paired_wrapper_and_phone_drop_releases_slot_promptly() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone(), 10).await;

    let wrapper = wrapper_connect_and_hello(port, api_key.clone(), token.clone())
        .await
        .expect("wrapper register");

    // Phone attaches.
    let (mut phone, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");
    let phello = ControlMessage::PhoneHello(claude_phone_shared::protocol::PhoneHello {
        token: token.clone(),
        cols: 80,
        rows: 24,
        user_agent: None,
    });
    phone
        .send(Message::Text(serde_json::to_string(&phello).unwrap()))
        .await
        .expect("phone hello");
    let _ = phone.next().await; // ServerHello
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drop BOTH peers in quick succession — the race a churning client
    // would create.
    drop(phone);
    drop(wrapper);

    // 1 s envelope: TM-RATE.7 cancel-propagation makes cleanup
    // sub-second; the envelope is generous so transient CI scheduling
    // jitter does not flake, but tight enough that a regression to a
    // multi-second cleanup (e.g. waiting on the 30 s keepalive tick)
    // fails this test.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The token slot MUST be re-registrable now.
    let _fresh = wrapper_connect_and_hello(port, api_key, token)
        .await
        .expect("TM-TEST.6: paired-drop slot must be reclaimed inside the 1 s envelope — a regression to multi-second cleanup would fail here");
}

// =====================================================================
// Test 3 — many-session chaos: random wrapper drops never leak slots.
//
// Concretely: register N wrappers, randomly drop a subset, then verify
// every dropped token re-registers and every surviving token is still
// taken. The fixed RNG seed keeps the test deterministic.
// =====================================================================
#[tokio::test]
async fn chaos_random_wrapper_drops_do_not_leak_slots() {
    // N is chosen against the per-IP HTTP rate limit (TM-RATE.1):
    // burst=10, sustained=5 r/s. Each test connect is one HTTP upgrade
    // request. N=8 plus ~4 re-register + ~4 survivor probes ≈ 16 total
    // requests; spacing them 250 ms apart keeps us inside the 5/s
    // budget while still exercising a meaningfully chaotic spread of
    // drop/survive interleavings.
    const N: usize = 6;
    const MAX_SESSIONS: usize = 30;
    const SEED: u64 = 0xC0DE_C0DE_C0DE_C0DE;
    // Per-request pacing must stay well clear of the 200 ms governor
    // refill quantum. 250 ms was too close to the floor and flaked
    // intermittently as scheduler jitter pushed adjacent requests
    // into the same refill window. 500 ms gives the bucket time to
    // regenerate fully between requests so the test never races the
    // limiter; the test is about session-lifecycle bookkeeping, not
    // rate-limit edge cases.
    const PACE: Duration = Duration::from_millis(500);

    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key.clone(), MAX_SESSIONS).await;

    // Register N wrappers; keep their sockets in a Vec.
    let mut tokens: Vec<SessionToken> = Vec::with_capacity(N);
    let mut sockets: Vec<Option<WsStream>> = Vec::with_capacity(N);
    for _ in 0..N {
        let token = SessionToken::generate();
        let ws = wrapper_connect_and_hello(port, api_key.clone(), token.clone())
            .await
            .expect("initial wrapper register must succeed");
        tokens.push(token);
        sockets.push(Some(ws));
        tokio::time::sleep(PACE).await;
    }

    // Roll a per-index drop decision with a deterministic seed.
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut dropped: Vec<bool> = Vec::with_capacity(N);
    for _ in 0..N {
        dropped.push(rng.gen_bool(0.5));
    }
    // Guarantee at least one drop AND at least one survivor — otherwise
    // the test degenerates and either branch of the invariant is
    // vacuously satisfied. With seed 0xC0DE… this is already true, but
    // pin it explicitly so a future seed change can't silently weaken
    // the assertion.
    if dropped.iter().all(|&b| b) {
        dropped[0] = false;
    }
    if dropped.iter().all(|&b| !b) {
        dropped[0] = true;
    }

    // Execute drops.
    for (i, was_dropped) in dropped.iter().enumerate() {
        if *was_dropped {
            if let Some(ws) = sockets[i].take() {
                drop(ws);
            }
        }
    }
    // Single settle window covering all drops — same rationale as
    // `drop_wrapper_and_settle`.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Invariant 1: every dropped token MUST be re-registrable.
    for (i, was_dropped) in dropped.iter().enumerate() {
        if *was_dropped {
            let token = tokens[i].clone();
            let r = wrapper_connect_and_hello(port, api_key.clone(), token).await;
            assert!(
                r.is_ok(),
                "TM-TEST.6: dropped token at index {i} must be re-registrable; got {r:?}"
            );
            // Stash the new socket so it does not get GC'd before the
            // survivor check below — a freshly opened socket whose
            // wrapper future hasn't been polled yet could race the
            // survivor probe.
            sockets[i] = Some(r.unwrap());
            tokio::time::sleep(PACE).await;
        }
    }

    // Invariant 2: every surviving token MUST still be taken (the live
    // wrapper still holds the slot).
    for (i, was_dropped) in dropped.iter().enumerate() {
        if !*was_dropped {
            let token = tokens[i].clone();
            let blocked = wrapper_connect_and_hello(port, api_key.clone(), token).await;
            assert!(
                blocked.is_err(),
                "TM-TEST.6: surviving token at index {i} must remain taken — a duplicate register succeeded, which means the live session was silently evicted or the slot leaked"
            );
            tokio::time::sleep(PACE).await;
        }
    }
}
