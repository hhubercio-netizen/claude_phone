use claude_phone_shared::SessionToken;
use claude_phone_wrapper::session::SessionState;

#[test]
fn default_is_unpaired() {
    let s = SessionState::default();
    assert!(s.token.is_none());
    assert!(s.public_url.is_none());
    assert!(!s.peer_connected);
}

#[test]
fn setting_token_marks_paired() {
    let mut s = SessionState::default();
    let t = SessionToken::generate();
    let t_str = t.as_str().to_string();
    s.token = Some(t);
    assert!(s.token.is_some());
    assert_eq!(s.token.as_ref().unwrap().as_str(), t_str);
}

#[test]
fn peer_connected_can_toggle() {
    let mut s = SessionState::default();
    assert!(!s.peer_connected);
    s.peer_connected = true;
    assert!(s.peer_connected);
    s.peer_connected = false;
    assert!(!s.peer_connected);
}

#[test]
fn clone_preserves_state() {
    let s = SessionState {
        token: Some(SessionToken::generate()),
        public_url: Some("https://example.com/s/x".into()),
        peer_connected: true,
    };
    let c = s.clone();
    assert!(c.token.is_some());
    assert_eq!(c.public_url.as_deref(), Some("https://example.com/s/x"));
    assert!(c.peer_connected);
}
