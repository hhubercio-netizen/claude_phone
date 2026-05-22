use claude_phone_gateway::session::SessionRegistry;
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
