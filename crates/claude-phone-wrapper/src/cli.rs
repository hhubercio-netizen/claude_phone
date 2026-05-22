use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about = "Wrap `claude` with phone bridging support")]
pub struct Cli {
    /// Path to wrapper config TOML.
    #[arg(short, long, env = "CLAUDE_PHONE_WRAPPER_CONFIG")]
    pub config: Option<PathBuf>,

    /// Override the `claude` binary path.
    #[arg(long, env = "CLAUDE_PHONE_CLAUDE_BIN", default_value = "claude")]
    pub claude_bin: String,

    /// All remaining args are forwarded to `claude` as-is.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub claude_args: Vec<String>,
}
