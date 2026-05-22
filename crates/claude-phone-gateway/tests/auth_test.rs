use claude_phone_gateway::auth::verify_api_key;
use claude_phone_shared::ApiKey;

#[test]
fn matches_known_key() {
    let k = ApiKey::generate();
    let allowed = vec![k.clone()];
    assert!(verify_api_key(&k, &allowed));
}

#[test]
fn rejects_unknown_key() {
    let k = ApiKey::generate();
    let other = ApiKey::generate();
    let allowed = vec![k];
    assert!(!verify_api_key(&other, &allowed));
}

#[test]
fn empty_allowlist_rejects() {
    let k = ApiKey::generate();
    assert!(!verify_api_key(&k, &[]));
}
