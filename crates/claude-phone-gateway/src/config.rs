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
    /// Expected `Origin` header on phone WebSocket upgrades. When `Some`, any
    /// browser-initiated WS that carries a different `Origin` is rejected
    /// with 403 — defense-in-depth against a malicious site opening WSes
    /// across origins should a token ever leak. When `None` (dev/tests),
    /// no Origin enforcement is performed. Production must set this to e.g.
    /// `"https://claude-phone.pl"`.
    #[serde(default)]
    pub public_origin: Option<String>,
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

// TM-CODE.6: documented operational ranges for config-loaded numeric fields.
// Below the min the service is functionally broken; above the max the values
// invite memory or runtime pathologies (e.g., Duration::from_secs(u64::MAX)
// effectively disables the sweeper; usize::MAX-sized DashMap defeats the
// memory cap). The gateway fails closed at load time rather than producing
// surprising runtime behaviour.
const MIN_SESSION_IDLE_SECS: u64 = 60;
const MAX_SESSION_IDLE_SECS: u64 = 30 * 24 * 60 * 60;
const MIN_MAX_SESSIONS: usize = 1;
const MAX_MAX_SESSIONS: usize = 10_000;

impl GatewayConfig {
    /// Backwards-compatible accessor returning the typed api keys. Kept so
    /// older call sites still compile; new code can read the field directly.
    pub fn parsed_api_keys(&self) -> anyhow::Result<Vec<ApiKey>> {
        Ok(self.api_keys.clone())
    }

    /// Validate config values that cannot be enforced by serde alone.
    /// Called automatically from [`load`]. Returns the first violation; if
    /// multiple fields are out of range, the operator fixes them one by one
    /// in subsequent restarts.
    pub fn validate(&self) -> anyhow::Result<()> {
        // TM-CODE.6: session_idle_timeout_secs in [60, 30 days].
        if self.session_idle_timeout_secs < MIN_SESSION_IDLE_SECS
            || self.session_idle_timeout_secs > MAX_SESSION_IDLE_SECS
        {
            anyhow::bail!(
                "session_idle_timeout_secs={} is outside the operational range \
                 [{}, {}] (TM-CODE.6). Refusing to start.",
                self.session_idle_timeout_secs,
                MIN_SESSION_IDLE_SECS,
                MAX_SESSION_IDLE_SECS
            );
        }
        // TM-CODE.6: max_sessions in [1, 10_000].
        if self.max_sessions < MIN_MAX_SESSIONS || self.max_sessions > MAX_MAX_SESSIONS {
            anyhow::bail!(
                "max_sessions={} is outside the operational range [{}, {}] \
                 (TM-CODE.6). Refusing to start.",
                self.max_sessions,
                MIN_MAX_SESSIONS,
                MAX_MAX_SESSIONS
            );
        }
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }
}
