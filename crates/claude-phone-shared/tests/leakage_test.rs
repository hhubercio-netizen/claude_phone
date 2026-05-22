use claude_phone_shared::{
    protocol::{ControlMessage, WrapperHello},
    ApiKey, SessionToken, TokenError,
};

#[test]
fn debug_does_not_print_session_token_value() {
    let t = SessionToken::generate();
    let s = format!("{:?}", t);
    assert_eq!(s, "SessionToken(***)");
    assert!(!s.contains(t.as_str()));
}

#[test]
fn debug_does_not_print_api_key_value() {
    let k = ApiKey::generate();
    let s = format!("{:?}", k);
    assert_eq!(s, "ApiKey(***)");
    assert!(!s.contains(k.as_str()));
}

#[test]
fn debug_wrapper_hello_does_not_leak_secrets() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let api_str = api_key.as_str().to_string();
    let token_str = token.as_str().to_string();

    let hello = WrapperHello {
        api_key,
        token,
        cols: 80,
        rows: 24,
        claude_version: None,
    };

    let s = format!("{:?}", hello);
    assert!(
        !s.contains(&api_str),
        "WrapperHello Debug leaked api_key value: {s}"
    );
    assert!(
        !s.contains(&token_str),
        "WrapperHello Debug leaked token value: {s}"
    );
}

#[test]
fn token_error_display_does_not_echo_input() {
    let bad = "definitely-not-43-chars";
    let err = SessionToken::parse(bad).unwrap_err();
    let s = format!("{}", err);
    assert!(
        !s.contains(bad),
        "TokenError Display echoed user input: {s}"
    );
}

#[test]
fn token_error_debug_is_opaque_variant() {
    let err = SessionToken::parse("x").unwrap_err();
    // Only one variant after M9.4 #5 collapse: pinning ensures the
    // refactor stays in place.
    assert!(matches!(err, TokenError::Invalid));
}

#[test]
fn control_message_serialized_json_does_not_unexpectedly_omit_token() {
    // Sanity: protocol JSON intentionally carries token/api_key fields
    // (they have to cross the wire). The point of this assertion is to
    // pin the field NAMES so a future serde rename doesn't silently
    // change the wire protocol.
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let msg = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"api_key\":"));
    assert!(json.contains("\"token\":"));
    assert!(json.contains(api_key.as_str()));
    assert!(json.contains(token.as_str()));
}
