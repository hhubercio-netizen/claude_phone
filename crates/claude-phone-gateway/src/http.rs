use std::sync::Arc;

use axum::routing::{any, get};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use claude_phone_shared::ApiKey;

use crate::config::GatewayConfig;
use crate::routes::{health, phone_ws, statics, wrapper_ws};
use crate::session::SessionRegistry;

pub fn build_app(config: &GatewayConfig) -> anyhow::Result<Router> {
    let registry = Arc::new(SessionRegistry::new(config.max_sessions));
    let allowed_keys = Arc::new(
        config
            .parsed_api_keys()?
            .into_iter()
            .collect::<Vec<ApiKey>>(),
    );

    let wrapper_state = wrapper_ws::WrapperWsState {
        registry: registry.clone(),
        allowed_keys,
    };

    let phone_state = phone_ws::PhoneWsState {
        registry: registry.clone(),
    };

    let static_state = statics::StaticsState {
        dir: config.static_dir.clone(),
    };

    let assets = ServeDir::new(config.static_dir.join("assets")).precompressed_gzip();

    let app = Router::new()
        .route("/healthz", get(health::handler))
        .route("/api/wrapper", any(wrapper_ws::handler))
        .with_state(wrapper_state)
        .route("/api/phone/:token", any(phone_ws::handler))
        .with_state(phone_state)
        .nest_service("/assets", assets)
        .route("/s/:token", get(statics::session_shell))
        .route("/", get(statics::root))
        .with_state(static_state)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    Ok(app)
}
