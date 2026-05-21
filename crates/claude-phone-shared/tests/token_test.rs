use claude_phone_shared::{ApiKey, SessionToken};

#[test]
fn session_token_generates_43_char_base64url() {
    let t = SessionToken::generate();
    assert_eq!(t.as_str().len(), 43);
    // base64url alphabet only
    assert!(t
        .as_str()
        .chars()
        .all(|c| { c.is_ascii_alphanumeric() || c == '-' || c == '_' }));
}

#[test]
fn session_token_two_generations_differ() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn session_token_parses_valid() {
    let t = SessionToken::generate();
    let s = t.as_str().to_string();
    let parsed = SessionToken::parse(&s).expect("valid token");
    assert_eq!(parsed.as_str(), s);
}

#[test]
fn session_token_rejects_too_short() {
    let r = SessionToken::parse("abc");
    assert!(r.is_err());
}

#[test]
fn session_token_rejects_invalid_chars() {
    let r = SessionToken::parse("!".repeat(43).as_str());
    assert!(r.is_err());
}

#[test]
fn session_token_constant_time_eq() {
    let a = SessionToken::generate();
    let b = SessionToken::parse(a.as_str()).unwrap();
    assert!(a.constant_time_eq(&b));
}

#[test]
fn api_key_generates_43_chars_base64url() {
    let k = ApiKey::generate();
    assert_eq!(k.as_str().len(), 43);
}

#[test]
fn token_serializes_as_string() {
    let t = SessionToken::generate();
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.starts_with('"') && json.ends_with('"'));
    let back: SessionToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t.as_str(), back.as_str());
}
