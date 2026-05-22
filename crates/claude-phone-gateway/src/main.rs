use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use claude_phone_gateway::{config::GatewayConfig, http::build_app, logging};

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

    let app = build_app(&config)?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutdown signal received");
}
