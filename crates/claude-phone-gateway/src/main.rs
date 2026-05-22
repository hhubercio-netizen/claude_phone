use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use claude_phone_gateway::{config::GatewayConfig, logging};

#[derive(Parser)]
#[command(version, about = "Claude Phone gateway server")]
struct Cli {
    /// Path to gateway TOML config.
    #[arg(
        short,
        long,
        env = "CLAUDE_PHONE_CONFIG",
        default_value = "config.toml"
    )]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = GatewayConfig::load(&cli.config)
        .with_context(|| format!("loading config {:?}", cli.config))?;

    logging::init(config.log_format);
    tracing::info!(bind = %config.bind_addr, "starting gateway");

    // HTTP/WS app comes in next tasks
    todo!("HTTP server")
}
