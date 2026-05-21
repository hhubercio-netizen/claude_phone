use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// 256-bit secret used in the URL handed to the phone.
/// Encoded as base64url without padding: exactly 43 characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionToken(String);

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("token must be exactly 43 base64url characters")]
    InvalidLength,
    #[error("token contains invalid characters")]
    InvalidChars,
}

impl SessionToken {
    pub const LEN: usize = 43;

    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn parse(s: &str) -> Result<Self, TokenError> {
        if s.len() != Self::LEN {
            return Err(TokenError::InvalidLength);
        }
        if !s.chars().all(is_base64url_char) {
            return Err(TokenError::InvalidChars);
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison to mitigate timing attacks.
    pub fn constant_time_eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in self.0.as_bytes().iter().zip(other.0.as_bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// API key used by wrappers to authenticate to the gateway.
/// Same format as SessionToken but semantically distinct.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiKey(String);

impl ApiKey {
    pub const LEN: usize = 43;

    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn parse(s: &str) -> Result<Self, TokenError> {
        if s.len() != Self::LEN {
            return Err(TokenError::InvalidLength);
        }
        if !s.chars().all(is_base64url_char) {
            return Err(TokenError::InvalidChars);
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn constant_time_eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in self.0.as_bytes().iter().zip(other.0.as_bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

fn is_base64url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}
