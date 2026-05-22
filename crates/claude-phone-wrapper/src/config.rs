use std::path::{Path, PathBuf};

use serde::Deserialize;

use claude_phone_shared::ApiKey;

#[derive(Debug, Clone, Deserialize)]
pub struct WrapperConfig {
    pub gateway_url: String,
    pub api_key: String,
    #[serde(default = "default_public_url_base")]
    pub public_url_base: String,
    #[serde(default = "default_rpc_bind")]
    pub rpc_bind: String,
}

fn default_public_url_base() -> String {
    "https://claude-phone.pl".into()
}

fn default_rpc_bind() -> String {
    "127.0.0.1:0".into()
}

impl WrapperConfig {
    pub fn parsed_api_key(&self) -> anyhow::Result<ApiKey> {
        ApiKey::parse(&self.api_key).map_err(|e| anyhow::anyhow!("invalid api_key: {e}"))
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.parsed_api_key()?;
        Ok(cfg)
    }

    pub fn default_path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "claude-phone")?;
        Some(dirs.config_dir().join("config.toml"))
    }
}
