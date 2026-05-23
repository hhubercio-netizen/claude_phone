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
