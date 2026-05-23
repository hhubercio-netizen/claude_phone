use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;

use claude_phone_gateway::{config::GatewayConfig, http::build_app, logging, serve};

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
    let listener = TcpListener::bind(config.bind_addr).await?;

    // TM-RATE.9 — serve::run replaces axum::serve so we can set
    // http1_header_read_timeout. The same function is exercised by
    // tests/rate_limit.rs, which keeps the slow-loris guard from
    // regressing if a future refactor reaches for axum::serve again.
    serve::run(listener, app, shutdown_signal()).await;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
}
