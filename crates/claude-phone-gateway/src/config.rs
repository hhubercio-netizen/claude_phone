use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

use claude_phone_shared::ApiKey;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub bind_addr: SocketAddr,
    pub static_dir: PathBuf,
    pub api_keys: Vec<String>, // raw strings, validated on load
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

fn default_session_timeout() -> u64 {
    300
}
fn default_max_sessions() -> usize {
    100
}

impl GatewayConfig {
    pub fn parsed_api_keys(&self) -> anyhow::Result<Vec<ApiKey>> {
        self.api_keys
            .iter()
            .map(|s| {
                ApiKey::parse(s).map_err(|e| anyhow::anyhow!("invalid api_key in config: {e}"))
            })
            .collect()
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.parsed_api_keys()?; // validate
        Ok(cfg)
    }
}
