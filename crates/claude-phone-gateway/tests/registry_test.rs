use std::time::Duration;

use claude_phone_gateway::session::{Frame, SessionRegistry};
use claude_phone_shared::SessionToken;

#[tokio::test]
async fn register_and_find() {
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _handle = reg.register_wrapper(token.clone()).await.unwrap();
    assert!(reg.lookup(&token).is_some());
}

#[tokio::test]
async fn register_twice_same_token_fails() {
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _first = reg.register_wrapper(token.clone()).await.unwrap();
    let second = reg.register_wrapper(token).await;
    assert!(second.is_err());
}

#[tokio::test]
async fn max_sessions_enforced() {
    let reg = SessionRegistry::new(1);
    let _first = reg
        .register_wrapper(SessionToken::generate())
        .await
        .unwrap();
    let second = reg.register_wrapper(SessionToken::generate()).await;
    assert!(second.is_err());
}

#[tokio::test]
async fn attach_phone_after_wrapper() {
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _w = reg.register_wrapper(token.clone()).await.unwrap();

    let phone_res = reg.attach_phone(&token).await;
    assert!(phone_res.is_ok());
}

#[tokio::test]
async fn attach_phone_to_missing_session() {
    let reg = SessionRegistry::new(10);
    let bad_token = SessionToken::generate();
    let res = reg.attach_phone(&bad_token).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn second_phone_attach_while_first_attached_fails() {
    // Sticky session is per-token, but only one phone can hold it at a time.
    // A second concurrent phone trying the same link is rejected so a token
    // leak doesn't let an attacker steal an active session.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _w = reg.register_wrapper(token.clone()).await.unwrap();
    let _first = reg.attach_phone(&token).await.unwrap();

    let second = reg.attach_phone(&token).await;
    assert!(second.is_err(), "second concurrent phone must be rejected");
}

#[tokio::test]
async fn buffered_binary_frames_replay_on_reattach() {
    // While no phone is attached, wrapper output piles into the gateway's
    // ring buffer. When a phone re-enters the link, the buffered chunks are
    // delivered before any new data, so the user sees the recent terminal
    // history instead of a blank screen.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let w = reg.register_wrapper(token.clone()).await.unwrap();

    // Simulate wrapper output landing while no phone is attached: push it
    // directly into PhoneChannel via the session's mutex.
    {
        let mut slot = w.session.to_phone.lock().await;
        slot.push_buffered(b"line-1 ".to_vec());
        slot.push_buffered(b"line-2".to_vec());
    }

    let mut phone = reg.attach_phone(&token).await.unwrap();
    // Frames should already be queued for the new phone's receiver.
    let f1 = phone.rx.recv().await.unwrap();
    let f2 = phone.rx.recv().await.unwrap();
    match (f1, f2) {
        (Frame::Binary(a), Frame::Binary(b)) => {
            assert_eq!(a, b"line-1 ");
            assert_eq!(b, b"line-2");
        }
        _ => panic!("expected two binary frames replayed in order"),
    }

    // After replay, no further frames should be queued (the buffer is drained).
    let next = tokio::time::timeout(std::time::Duration::from_millis(50), phone.rx.recv()).await;
    assert!(next.is_err(), "no extra frames expected after replay");
}

#[tokio::test]
async fn sweep_drops_phone_idle_session() {
    // No phone has ever attached, idle window has elapsed → session goes.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _w = reg.register_wrapper(token.clone()).await.unwrap();

    // Wait past the idle window. 50ms is enough to clear duration_since on
    // any reasonable scheduler.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let dropped = reg.sweep_expired(Duration::from_millis(10)).await;
    assert_eq!(dropped, 1);
    assert!(reg.lookup(&token).is_none());
}

#[tokio::test]
async fn sweep_keeps_session_while_phone_attached() {
    // Even if "last seen" is ancient, an actively-attached phone keeps the
    // session alive — we don't want to nuke a long-lived terminal that the
    // user is currently looking at.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let _w = reg.register_wrapper(token.clone()).await.unwrap();
    let _p = reg.attach_phone(&token).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    let dropped = reg.sweep_expired(Duration::from_millis(10)).await;
    assert_eq!(dropped, 0);
    assert!(reg.lookup(&token).is_some());
}

#[tokio::test]
async fn sweep_keeps_session_when_phone_just_left() {
    // Phone attached, then detached. Sweep within the idle window must
    // preserve the session so a reconnect can land on the same token.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let w = reg.register_wrapper(token.clone()).await.unwrap();
    let _p = reg.attach_phone(&token).await.unwrap();
    {
        let mut slot = w.session.to_phone.lock().await;
        slot.detach();
    }
    w.session.touch_phone().await;

    // Run sweep immediately — last_phone_seen was just refreshed.
    let dropped = reg.sweep_expired(Duration::from_secs(60)).await;
    assert_eq!(dropped, 0);
    assert!(reg.lookup(&token).is_some());
}

#[tokio::test]
async fn drop_session_fires_cancel() {
    // The sweeper relies on this: dropping a session must notify any
    // WS task awaiting `cancel.notified()` so they tear down promptly.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let w = reg.register_wrapper(token.clone()).await.unwrap();
    let cancel = w.session.cancel.clone();

    // Drop FIRST, then spawn the waiter. This exercises the sticky-cancel
    // case: even when a task starts polling AFTER drop_session has fired,
    // it still observes cancellation (because the AtomicBool flag is set).
    reg.drop_session(&token).await;
    let waiter = tokio::spawn(async move {
        cancel.cancelled().await;
    });
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("drop_session did not fire cancel")
        .unwrap();
    assert!(reg.lookup(&token).is_none());
}

#[tokio::test]
async fn detach_then_reattach_after_disconnect() {
    // Phone leaves (detach), then a new phone arrives on the same token.
    // The second attach must succeed — that's the whole point of sticky
    // sessions.
    let reg = SessionRegistry::new(10);
    let token = SessionToken::generate();
    let w = reg.register_wrapper(token.clone()).await.unwrap();

    let _first = reg.attach_phone(&token).await.unwrap();
    {
        let mut slot = w.session.to_phone.lock().await;
        slot.detach();
    }

    let second = reg.attach_phone(&token).await;
    assert!(second.is_ok(), "reattach after detach must succeed");
}
