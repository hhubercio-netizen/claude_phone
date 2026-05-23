use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

use claude_phone_shared::ApiKey;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub bind_addr: SocketAddr,
    pub static_dir: PathBuf,
    /// Typed so that any accidental `Debug`-print of the config redacts the
    /// secret values (`[ApiKey(***), ...]`) instead of leaking them into
    /// logs. TOML deserialization runs through `ApiKey::TryFrom<String>`
    /// so malformed entries are rejected at load time.
    pub api_keys: Vec<ApiKey>,
    #[serde(default = "default_session_timeout")]
    pub session_idle_timeout_secs: u64,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    #[serde(default)]
    pub log_format: LogFormat,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

/// Default phone-idle timeout: 7 days. The session lives as long as the
/// phone has been seen within this window. Resets on phone attach (sticky
/// session). Generous default — most users will pair on Monday and expect
/// the link to still work on Friday. If the wrapper exits or a new /pair
/// is triggered on the host, the session is dropped immediately regardless
/// of this timeout.
fn default_session_timeout() -> u64 {
    7 * 24 * 60 * 60
}
fn default_max_sessions() -> usize {
    100
}

impl GatewayConfig {
    /// Backwards-compatible accessor returning the typed api keys. Kept so
    /// older call sites still compile; new code can read the field directly.
    pub fn parsed_api_keys(&self) -> anyhow::Result<Vec<ApiKey>> {
        Ok(self.api_keys.clone())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        Ok(cfg)
    }
}
