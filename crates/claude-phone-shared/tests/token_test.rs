use claude_phone_shared::{ApiKey, SessionToken, TokenError};

#[test]
fn session_token_generates_43_char_base64url() {
    let t = SessionToken::generate();
    assert_eq!(t.as_str().len(), 43);
    assert!(t
        .as_str()
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
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
fn session_token_rejects_too_short_42() {
    let r = SessionToken::parse(&"a".repeat(42));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_too_long_44() {
    let r = SessionToken::parse(&"a".repeat(44));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_empty() {
    let r = SessionToken::parse("");
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_padded_base64() {
    // 44 chars including '=' padding — wrong length AND invalid char
    let r = SessionToken::parse(&format!("{}=", "a".repeat(43)));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_invalid_chars() {
    let r = SessionToken::parse(&"!".repeat(43));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_ct_eq_equal_case() {
    let a = SessionToken::generate();
    let b = SessionToken::parse(a.as_str()).unwrap();
    assert!(a.ct_eq(&b));
}

#[test]
fn session_token_ct_eq_inequality_case() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert!(!a.ct_eq(&b));
}

#[test]
fn session_token_serde_roundtrip() {
    let t = SessionToken::generate();
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.starts_with('"') && json.ends_with('"'));
    let back: SessionToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t.as_str(), back.as_str());
}

#[test]
fn session_token_serde_rejects_short_string() {
    let json = r#""abc""#;
    let r: Result<SessionToken, _> = serde_json::from_str(json);
    assert!(r.is_err(), "serde must re-validate via TryFrom");
}

#[test]
fn api_key_generates_43_chars_base64url() {
    let k = ApiKey::generate();
    assert_eq!(k.as_str().len(), 43);
    assert!(k
        .as_str()
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}

#[test]
fn api_key_parses_valid() {
    let k = ApiKey::generate();
    let s = k.as_str().to_string();
    let parsed = ApiKey::parse(&s).expect("valid api_key");
    assert_eq!(parsed.as_str(), s);
}

#[test]
fn api_key_rejects_too_short() {
    let r = ApiKey::parse("abc");
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn api_key_ct_eq_equal_and_unequal() {
    let a = ApiKey::generate();
    let b = ApiKey::parse(a.as_str()).unwrap();
    let c = ApiKey::generate();
    assert!(a.ct_eq(&b));
    assert!(!a.ct_eq(&c));
}

#[test]
fn api_key_serde_roundtrip() {
    let k = ApiKey::generate();
    let json = serde_json::to_string(&k).unwrap();
    let back: ApiKey = serde_json::from_str(&json).unwrap();
    assert_eq!(k.as_str(), back.as_str());
}

#[test]
fn api_key_serde_rejects_invalid_string() {
    let json = r#""not-a-real-key""#;
    let r: Result<ApiKey, _> = serde_json::from_str(json);
    assert!(r.is_err(), "serde must re-validate via TryFrom");
}
