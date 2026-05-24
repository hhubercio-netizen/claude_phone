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
    ///
    /// TM-SECRET.12: gateway-dev.toml ships a deliberately too-short
    /// placeholder; `ApiKey::TryFrom<String>` rejects it on parse, so a
    /// gateway accidentally pointed at the dev config fails to start
    /// instead of silently accepting an attacker-known key.
    pub api_keys: Vec<ApiKey>,
    #[serde(default = "default_session_timeout")]
    pub session_idle_timeout_secs: u64,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    #[serde(default)]
    pub log_format: LogFormat,
    /// TM-WS.9 — deployment environment. Default is `Development` so dev
    /// and test configs continue to boot without an explicit setting. A
    /// production deployment must declare `environment = "production"` in
    /// `/etc/claude-phone/gateway.toml` so `validate()` enforces the
    /// `public_origin` invariant.
    #[serde(default)]
    pub environment: Environment,
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

/// Deployment environment. Drives boot-time invariants that only matter
/// in production — currently the TM-WS.9 fail-loud on missing
/// `public_origin`. Default is `Development` so an existing dev or test
/// config that omits the field continues to boot unchanged.
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    #[default]
    Development,
    Production,
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
        // TM-WS.9: production must declare its public origin so that the
        // Origin defense (TM-WS.1, .2, .3) actually fires. A misconfigured
        // production gateway is detected at startup rather than after the
        // first malicious request reaches the WS handler.
        if matches!(self.environment, Environment::Production) && self.public_origin.is_none() {
            anyhow::bail!(
                "production environment requires public_origin to be set in \
                 gateway.toml (TM-WS.9). Refusing to start."
            );
        }
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        // TM-SECRET.1: fail-loud on a gateway config that any non-owner /
        // non-group reader could open. The file carries every accepted
        // `ApiKey` for the deployment — the shared secret with every wrapper
        // — so on a multi-tenant host (VPS with a second account, shared
        // dev box, leaked CI runner) a world- or group-writable mode would
        // hand impersonation to anyone who can read it. The target deploy
        // mode is `0640 root:claude-phone`; the mask therefore rejects any
        // world bit AND group-write, while still permitting the service
        // account group to read. Stricter `0600` and `0400` also pass — a
        // single-user host that runs the gateway as a non-root user can
        // tighten further without breaking this gate. Windows relies on
        // the default profile ACL and skips the check (mirrors the wrapper
        // pattern at `claude-phone-wrapper/src/config.rs::load`).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(path)?;
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o027 != 0 {
                anyhow::bail!(
                    "gateway config {path:?} has permissive mode {mode:#o}; \
                     contains api_keys, must be at most 0640 with no \
                     world bits and no group-write (TM-SECRET.1). \
                     Run: chmod 640 {path:?} && chown root:claude-phone {path:?}"
                );
            }
        }
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }
}
