use claude_phone_shared::protocol::*;
use claude_phone_shared::{ApiKey, SessionToken};

#[test]
fn wrapper_hello_roundtrip() {
    let msg = ControlMessage::WrapperHello(WrapperHello {
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
        claude_version: Some("1.2.3".into()),
    });

    let s = serde_json::to_string(&msg).unwrap();
    assert!(s.contains("wrapper_hello"));

    let back: ControlMessage = serde_json::from_str(&s).unwrap();
    match back {
        ControlMessage::WrapperHello(h) => {
            assert_eq!(h.cols, 80);
            assert_eq!(h.rows, 24);
            assert_eq!(h.claude_version.as_deref(), Some("1.2.3"));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn phone_hello_roundtrip() {
    let msg = ControlMessage::PhoneHello(PhoneHello {
        token: SessionToken::generate(),
        cols: 40,
        rows: 80,
        user_agent: Some("Mozilla/5.0".into()),
    });

    let s = serde_json::to_string(&msg).unwrap();
    assert!(s.contains("phone_hello"));

    let back: ControlMessage = serde_json::from_str(&s).unwrap();
    match back {
        ControlMessage::PhoneHello(h) => {
            assert_eq!(h.cols, 40);
            assert_eq!(h.rows, 80);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn server_hello_ok() {
    let msg = ControlMessage::ServerHello(ServerHello {
        session_id: "abc123".into(),
        peer_connected: false,
    });
    let s = serde_json::to_string(&msg).unwrap();
    assert!(s.contains("server_hello"));
    let _: ControlMessage = serde_json::from_str(&s).unwrap();
}

#[test]
fn error_message() {
    let msg = ControlMessage::Error(ErrorMessage {
        code: ErrorCode::InvalidToken,
        message: "token not found".into(),
    });
    let s = serde_json::to_string(&msg).unwrap();
    assert!(s.contains("invalid_token"));
    let _: ControlMessage = serde_json::from_str(&s).unwrap();
}

#[test]
fn resize_roundtrip() {
    let msg = ControlMessage::Resize(Resize {
        cols: 100,
        rows: 50,
    });
    let s = serde_json::to_string(&msg).unwrap();
    let back: ControlMessage = serde_json::from_str(&s).unwrap();
    match back {
        ControlMessage::Resize(r) => {
            assert_eq!(r.cols, 100);
            assert_eq!(r.rows, 50);
        }
        _ => panic!("wrong"),
    }
}

#[test]
fn peer_status() {
    let msg = ControlMessage::PeerStatus(PeerStatus { connected: true });
    let s = serde_json::to_string(&msg).unwrap();
    let _: ControlMessage = serde_json::from_str(&s).unwrap();
}

#[test]
fn close_roundtrip() {
    let msg = ControlMessage::Close(Close {
        reason: Some("user quit".into()),
    });
    let s = serde_json::to_string(&msg).unwrap();
    let _: ControlMessage = serde_json::from_str(&s).unwrap();
}

#[test]
fn unknown_type_rejected() {
    let s = r#"{"type":"bogus","x":1}"#;
    let r: Result<ControlMessage, _> = serde_json::from_str(s);
    assert!(r.is_err());
}
