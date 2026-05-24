//! TM-TEST.2 — property-based tests for the wire protocol and the
//! secret-token parsers.
//!
//! Hand-written tests in `protocol_test.rs` and `token_test.rs` cover the
//! known fixtures (80x24 resize, the "first 42 chars" rejection, etc.).
//! These tests cover the *contract*:
//!
//!   1. `SessionToken::parse` and `ApiKey::parse` are total functions —
//!      any input returns Ok or Err, never panics — and Ok holds iff the
//!      strict 43-character base64url invariant holds. The two types must
//!      behave identically on every input (so any future drift in either
//!      validator is caught immediately).
//!
//!   2. `SessionToken::generate()` and `ApiKey::generate()` produce a
//!      value that `parse` accepts as identical (constant-time equality).
//!      The generator and validator stay coupled: a change to either side
//!      that breaks the round-trip fails this test.
//!
//!   3. `ControlMessage` JSON round-trip is the identity for every
//!      variant whose fields we can synthesize, across the full u16 /
//!      bool / Option<String> domain. Catches the regression where
//!      someone introduces a width cap and silently truncates u16::MAX,
//!      or where a `#[serde(rename)]` is added on one side only.

use claude_phone_shared::protocol::{
    Close, ControlMessage, PeerStatus, PhoneHello, Resize, ServerHello, WrapperHello,
};
use claude_phone_shared::{ApiKey, SessionToken, TokenError};
use proptest::prelude::*;

fn is_base64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

proptest! {
    /// TM-TEST.2 — `SessionToken::parse` / `ApiKey::parse` never panic and
    /// return Ok iff length == 43 AND every byte is in `[A-Za-z0-9_\-]`.
    /// Strategy is "any printable string up to 200 chars" — most inputs
    /// will fail the length check, exercising the Err path; the rare hit
    /// on length-43 covers the charset arm.
    #[test]
    fn prop_secret_token_parse_contract(s in "\\PC{0,200}") {
        let charset_ok = s.bytes().all(is_base64url_byte);
        let length_ok = s.len() == 43;

        let session = SessionToken::parse(&s);
        let api = ApiKey::parse(&s);

        match (length_ok && charset_ok, &session) {
            (true, Ok(t)) => prop_assert_eq!(t.as_str(), s.as_str()),
            (false, Err(TokenError::Invalid)) => {}
            (true, Err(_)) => prop_assert!(false, "valid input rejected: {:?}", s),
            (false, Ok(_)) => prop_assert!(false, "invalid input accepted: {:?}", s),
        }
        // Symmetry: ApiKey and SessionToken share the same validator, so
        // they must agree on every input. A regression that diverges them
        // (e.g. a future change that adds a charset check to one side only)
        // is caught here.
        prop_assert_eq!(session.is_ok(), api.is_ok(),
            "ApiKey / SessionToken parse disagree on {:?}", s);
    }

    /// TM-TEST.2 — `SessionToken::generate()` and `ApiKey::generate()`
    /// produce a value that `parse` accepts and `ct_eq` confirms equal.
    /// The iteration count is the implicit input — proptest runs the body
    /// many times, exercising fresh OS-CSPRNG draws each time.
    #[test]
    fn prop_secret_token_generate_roundtrip(_iter in 0u32..256) {
        let t = SessionToken::generate();
        let s = t.as_str().to_string();
        prop_assert_eq!(s.len(), SessionToken::LEN);
        let t2 = SessionToken::parse(&s).expect("generated SessionToken must parse");
        prop_assert!(t.ct_eq(&t2));

        let a = ApiKey::generate();
        let s = a.as_str().to_string();
        prop_assert_eq!(s.len(), ApiKey::LEN);
        let a2 = ApiKey::parse(&s).expect("generated ApiKey must parse");
        prop_assert!(a.ct_eq(&a2));
    }

    /// TM-TEST.2 — wire-protocol JSON round-trip is the identity for every
    /// `ControlMessage` variant we can synthesize. Boundary values for
    /// `cols`/`rows` (0, u16::MAX) and option presence are explored by
    /// proptest's shrinking, which our hand-written fixtures cannot reach.
    #[test]
    fn prop_control_message_roundtrip(
        cols in any::<u16>(),
        rows in any::<u16>(),
        connected in any::<bool>(),
        reason in proptest::option::of("[ -~]{0,80}"),
        session_id in "[A-Za-z0-9_-]{1,32}",
        peer_connected in any::<bool>(),
        user_agent in proptest::option::of("[ -~]{0,80}"),
        variant in 0u8..6,
    ) {
        let token = SessionToken::generate();
        let api_key = ApiKey::generate();

        let msg = match variant {
            0 => ControlMessage::Resize(Resize { cols, rows }),
            1 => ControlMessage::PeerStatus(PeerStatus { connected }),
            2 => ControlMessage::Close(Close { reason }),
            3 => ControlMessage::ServerHello(ServerHello { session_id, peer_connected }),
            4 => ControlMessage::WrapperHello(WrapperHello { api_key, token, cols, rows }),
            _ => ControlMessage::PhoneHello(PhoneHello { token, cols, rows, user_agent }),
        };

        let json = serde_json::to_string(&msg)
            .expect("serializing a constructed ControlMessage must succeed");
        let back: ControlMessage = serde_json::from_str(&json)
            .expect("round-tripping our own JSON must succeed");

        match (&msg, &back) {
            (ControlMessage::Resize(a), ControlMessage::Resize(b)) => {
                prop_assert_eq!(a.cols, b.cols);
                prop_assert_eq!(a.rows, b.rows);
            }
            (ControlMessage::PeerStatus(a), ControlMessage::PeerStatus(b)) => {
                prop_assert_eq!(a.connected, b.connected);
            }
            (ControlMessage::Close(a), ControlMessage::Close(b)) => {
                prop_assert_eq!(&a.reason, &b.reason);
            }
            (ControlMessage::ServerHello(a), ControlMessage::ServerHello(b)) => {
                prop_assert_eq!(&a.session_id, &b.session_id);
                prop_assert_eq!(a.peer_connected, b.peer_connected);
            }
            (ControlMessage::WrapperHello(a), ControlMessage::WrapperHello(b)) => {
                prop_assert!(a.api_key.ct_eq(&b.api_key));
                prop_assert!(a.token.ct_eq(&b.token));
                prop_assert_eq!(a.cols, b.cols);
                prop_assert_eq!(a.rows, b.rows);
            }
            (ControlMessage::PhoneHello(a), ControlMessage::PhoneHello(b)) => {
                prop_assert!(a.token.ct_eq(&b.token));
                prop_assert_eq!(a.cols, b.cols);
                prop_assert_eq!(a.rows, b.rows);
                prop_assert_eq!(&a.user_agent, &b.user_agent);
            }
            _ => prop_assert!(false, "ControlMessage variant changed across JSON round-trip"),
        }
    }
}
