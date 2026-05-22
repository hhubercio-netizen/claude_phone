use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::config::LogFormat;

pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,claude_phone_gateway=debug"));

    let registry = tracing_subscriber::registry().with(filter);

    match format {
        LogFormat::Pretty => {
            registry.with(fmt::layer().pretty()).init();
        }
        LogFormat::Json => {
            registry.with(fmt::layer().json()).init();
        }
    }
}
