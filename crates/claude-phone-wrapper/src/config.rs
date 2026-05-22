use std::path::{Path, PathBuf};

use serde::Deserialize;

use claude_phone_shared::ApiKey;

#[derive(Debug, Clone, Deserialize)]
pub struct WrapperConfig {
    pub gateway_url: String,
    /// Typed so that any accidental `Debug`-print of the config redacts the
    /// secret value (`ApiKey(***)`) instead of leaking it into logs.
    /// TOML deserialization runs through `ApiKey::TryFrom<String>` so
    /// malformed values are rejected at load time.
    pub api_key: ApiKey,
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
    /// Backwards-compatible accessor returning the typed api key. Kept so
    /// older call sites that called `parsed_api_key()` still compile; new
    /// code can read the field directly.
    pub fn parsed_api_key(&self) -> anyhow::Result<ApiKey> {
        Ok(self.api_key.clone())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        Ok(cfg)
    }

    pub fn default_path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "claude-phone")?;
        Some(dirs.config_dir().join("config.toml"))
    }
}
