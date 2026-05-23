// TM-CODE.4 — regression test for SessionRegistry shard-race over-allocation.
//
// Before the atomic-reservation fix in `register_wrapper`, two concurrent
// register calls whose tokens hashed to different DashMap shards could both
// pass the soft `len() < max_sessions` pre-check and both insert, leaving the
// registry one (or more) sessions above the documented cap. The fix replaces
// the soft pre-check with an atomic counter (`active_count: AtomicUsize`) that
// is incremented before any per-session state is allocated.
//
// This test spawns 50 parallel `register_wrapper` calls with 50 distinct
// tokens against a registry whose `max_sessions = 5`. After the fix, exactly
// 5 must succeed and 45 must return `Err(max sessions reached)`.

use std::sync::Arc;

use claude_phone_gateway::session::SessionRegistry;
use claude_phone_shared::SessionToken;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn registry_enforces_max_sessions_under_concurrent_registration() {
    let registry = Arc::new(SessionRegistry::new(5));

    let tokens: Vec<SessionToken> = (0..50).map(|_| SessionToken::generate()).collect();

    let mut handles = Vec::with_capacity(50);
    for token in tokens {
        let r = registry.clone();
        handles.push(tokio::spawn(async move { r.register_wrapper(token).await }));
    }

    let mut ok_count = 0usize;
    let mut err_count = 0usize;
    for h in handles {
        match h.await.expect("task did not panic") {
            Ok(_) => ok_count += 1,
            Err(_) => err_count += 1,
        }
    }

    assert_eq!(
        ok_count, 5,
        "exactly max_sessions=5 must succeed; got ok={} err={}",
        ok_count, err_count
    );
    assert_eq!(err_count, 45);
    assert_eq!(registry.len(), 5);
}

#[tokio::test]
async fn registry_active_count_decrements_on_remove() {
    // TM-CODE.4 — drop_session and remove must decrement the atomic counter
    // so the registry can be re-filled to the cap after sessions go away.
    let registry = SessionRegistry::new(2);
    let t1 = SessionToken::generate();
    let t2 = SessionToken::generate();
    let t3 = SessionToken::generate();

    let _h1 = registry.register_wrapper(t1.clone()).await.unwrap();
    let _h2 = registry.register_wrapper(t2.clone()).await.unwrap();

    // Third registration must fail (cap = 2).
    assert!(registry.register_wrapper(t3.clone()).await.is_err());

    // Drop one and verify a new registration succeeds.
    registry.drop_session(&t1).await;
    assert!(registry.register_wrapper(t3).await.is_ok());

    // Sanity: `remove` is idempotent and only decrements on actual removal.
    registry.remove(&t2);
    registry.remove(&t2); // second call is a no-op
    assert_eq!(registry.len(), 1);
}
