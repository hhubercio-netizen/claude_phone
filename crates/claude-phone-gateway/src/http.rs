use std::sync::Arc;

use axum::http::{header, HeaderName, HeaderValue, Request};
use axum::routing::{any, get};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use claude_phone_shared::ApiKey;

use crate::config::GatewayConfig;
use crate::routes::{health, phone_ws, statics, wrapper_ws};
use crate::session::SessionRegistry;

/// Redact session tokens from a path so they never appear in logs.
/// Tokens live in `/s/<token>` and `/api/phone/<token>`.
pub(crate) fn redact_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("/s/") {
        let tail = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
        return format!("/s/<redacted>{tail}");
    }
    if let Some(rest) = path.strip_prefix("/api/phone/") {
        let tail = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
        return format!("/api/phone/<redacted>{tail}");
    }
    path.to_string()
}

pub fn build_app(config: &GatewayConfig) -> anyhow::Result<Router> {
    let registry = Arc::new(SessionRegistry::new(config.max_sessions));
    let allowed_keys: Arc<Vec<ApiKey>> = Arc::new(config.api_keys.clone());

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

    // TraceLayer with a span builder that redacts session tokens from the URI
    // so they never land in pretty/JSON logs. Tokens are bearer-equivalent.
    let trace_layer = TraceLayer::new_for_http().make_span_with(|req: &Request<_>| {
        let method = req.method();
        let redacted = redact_path(req.uri().path());
        tracing::info_span!("http", %method, path = %redacted)
    });

    // Security headers applied to ALL responses. Static content is same-origin
    // only and never embeds remote scripts, so CSP can be strict.
    let security_headers = tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self'; \
                 style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; \
                 font-src 'self' data:; \
                 connect-src 'self' ws: wss:; \
                 frame-ancestors 'none'; \
                 base-uri 'self'; \
                 form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ));

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
        .layer(security_headers)
        .layer(CompressionLayer::new())
        .layer(trace_layer);

    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::redact_path;

    #[test]
    fn redacts_session_shell_token() {
        assert_eq!(
            redact_path("/s/abcdefghijabcdefghijabcdefghijabcdefghijabc"),
            "/s/<redacted>"
        );
    }

    #[test]
    fn redacts_phone_ws_token() {
        assert_eq!(
            redact_path("/api/phone/abcdefghijabcdefghijabcdefghijabcdefghijabc"),
            "/api/phone/<redacted>"
        );
    }

    #[test]
    fn preserves_tail_after_token() {
        assert_eq!(
            redact_path("/s/sometoken/extra/path"),
            "/s/<redacted>/extra/path"
        );
    }

    #[test]
    fn leaves_other_paths_unchanged() {
        assert_eq!(redact_path("/healthz"), "/healthz");
        assert_eq!(redact_path("/assets/main.js"), "/assets/main.js");
        assert_eq!(redact_path("/"), "/");
    }
}
