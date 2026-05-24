//! Control-plane wire types shared by wrapper and gateway.
//!
//! # JSON parsing depth (TM-INPUT.5)
//!
//! All `ControlMessage` parsing uses `serde_json::from_str` with the crate
//! default recursion limit of 128 levels. The default is preserved
//! intentionally — no caller in the workspace enables the `unbounded_depth`
//! cargo feature, and the forward-looking tests `rejects_deeply_nested_json`
//! / `rejects_deeply_nested_control_message` below catch any future drift
//! (e.g. a transitive crate that flips on `unbounded_depth`, or a refactor
//! that swaps in a custom `Deserializer` without re-setting the limit).
//! Without the cap, an attacker sending arbitrarily deep `{"a":{"a":…}}`
//! frames could stack-overflow the gateway parser thread.

use serde::{Deserialize, Serialize};

use crate::{ApiKey, SessionToken};

/// All control messages sent over WebSocket as JSON text frames.
/// PTY data is sent as binary frames and has no enum variant here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    WrapperHello(WrapperHello),
    PhoneHello(PhoneHello),
    ServerHello(ServerHello),
    Error(ErrorMessage),
    Resize(Resize),
    PeerStatus(PeerStatus),
    Close(Close),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperHello {
    pub api_key: ApiKey,
    pub token: SessionToken,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneHello {
    pub token: SessionToken,
    pub cols: u16,
    pub rows: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHello {
    pub session_id: String,
    pub peer_connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessage {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidToken,
    InvalidApiKey,
    SessionTaken,
    Expired,
    Internal,
    ProtocolViolation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Resize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PeerStatus {
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Close {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // TM-INPUT.5: forward-looking depth guards. The serde_json default
    // recursion limit (128) must keep rejecting deeply nested input — the
    // assertions below break if anyone enables `serde_json/unbounded_depth`
    // or swaps in a custom `Deserializer` without resetting `set_max_depth`.

    #[test]
    fn rejects_deeply_nested_json() {
        // 200 levels of `{"a":` nested — exceeds serde_json's 128 default.
        let mut s = String::new();
        for _ in 0..200 {
            s.push_str("{\"a\":");
        }
        s.push('1');
        for _ in 0..200 {
            s.push('}');
        }
        let result: Result<Value, _> = serde_json::from_str(&s);
        assert!(
            result.is_err(),
            "200-deep JSON must reject; the serde_json default cap is 128",
        );
    }

    #[test]
    fn rejects_deeply_nested_control_message() {
        // Same shape but typed at ControlMessage. Even if a future
        // ControlMessage variant gained a recursive field, this test
        // still catches an `unbounded_depth` regression — the parser hits
        // the depth limit before ever inspecting the type's fields.
        let mut s = String::new();
        s.push_str("{\"type\":\"phone_hello\",\"token\":\"x\",\"cols\":80,\"rows\":24");
        for _ in 0..200 {
            s.push_str(",\"x\":{");
        }
        let result: Result<ControlMessage, _> = serde_json::from_str(&s);
        assert!(
            result.is_err(),
            "200-deep ControlMessage payload must reject before reaching field validation",
        );
    }
}
