use std::sync::Arc;

use axum::http::{header, HeaderName, HeaderValue, Request};
use axum::routing::{any, get};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use claude_phone_shared::ApiKey;

use crate::config::GatewayConfig;
use crate::rate_limit::{AuthRateLimiter, PER_IP_BURST, PER_IP_REQ_PER_SEC};
use crate::routes::{health, phone_ws, statics, wrapper_ws};
use crate::session::SessionRegistry;

/// Redact session tokens from a path so they never appear in logs.
/// Tokens live in `/s/<token>` and `/api/phone/<token>`.
// TM-AUTH.10: every tracing span on the HTTP layer goes through this so the
// raw `/s/<token>` and `/api/phone/<token>` paths are replaced with the
// `<redacted>` placeholder before they hit any sink (stdout, JSON, file).
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

    // Phone-idle sweeper. Runs for the lifetime of the process; drops any
    // session whose phone has been gone for >= session_idle_timeout_secs.
    // Sweep interval is min(60s, timeout/4) so short timeouts (tests, dev)
    // still get acted on promptly.
    // TM-AUTH.11: sweep interval bound = min(60s, timeout/4). A 7-day default
    // timeout sweeps once a minute; tests with sub-minute timeouts sweep at
    // timeout/4 so a forgotten phone session can never linger past 1.25×
    // its declared idle ceiling.
    let idle_timeout = std::time::Duration::from_secs(config.session_idle_timeout_secs);
    let sweep_interval = std::cmp::min(
        std::time::Duration::from_secs(60),
        std::cmp::max(std::time::Duration::from_secs(1), idle_timeout / 4),
    );
    {
        let registry = registry.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(sweep_interval);
            ticker.tick().await; // skip immediate tick
            loop {
                ticker.tick().await;
                let dropped = registry.sweep_expired(idle_timeout).await;
                if dropped > 0 {
                    tracing::info!(dropped, "idle sweeper dropped expired sessions");
                }
            }
        });
    }

    // TM-RATE.2 — single process-wide AuthRateLimiter so failures from the
    // same IP add up across concurrent wrapper attempts. Wrapped in Clone
    // (cheap Arc clone) and stashed on the per-handler state.
    let auth_rate_limiter = AuthRateLimiter::new();

    let wrapper_state = wrapper_ws::WrapperWsState {
        registry: registry.clone(),
        allowed_keys,
        public_origin: config.public_origin.clone(),
        auth_rate_limiter,
    };

    let phone_state = phone_ws::PhoneWsState {
        registry: registry.clone(),
        public_origin: config.public_origin.clone(),
    };

    let static_state = statics::StaticsState {
        dir: config.static_dir.clone(),
    };

    // TM-INPUT.6: tower-http's ServeDir canonicalizes the request path and
    // rejects any traversal that escapes the configured root. Verified by
    // `tests/path_traversal_test.rs::serve_dir_rejects_*`. A future swap to
    // a hand-rolled file server would have to re-prove these tests; the
    // forward-looking suite is the only thing that catches the regression.
    let assets = ServeDir::new(config.static_dir.join("assets")).precompressed_gzip();

    // TraceLayer with a span builder that redacts session tokens from the URI
    // so they never land in pretty/JSON logs. Tokens are bearer-equivalent.
    let trace_layer = TraceLayer::new_for_http().make_span_with(|req: &Request<_>| {
        let method = req.method();
        let redacted = redact_path(req.uri().path());
        tracing::info_span!("http", %method, path = %redacted)
    });

    // Security headers applied to ALL responses. Static content is same-origin
    // only and never embeds remote scripts, so CSP can be strict. `connect-src
    // 'self'` is sufficient for the page's same-host WebSocket — modern
    // browsers (Chrome, Firefox, Safari) match wss to the document's host
    // under 'self' even though the spec arguably wouldn't because scheme
    // differs from https. If you ever serve from a different host for the
    // WS endpoint, add `wss://<host>` explicitly here.
    let security_headers = tower::ServiceBuilder::new()
        // TM-FRONT.6: Content-Security-Policy locked to same-origin assets.
        // `default-src 'self'` + an empty `object-src 'none'` block plugin
        // surfaces; `frame-ancestors 'none'` denies framing; `base-uri`
        // and `form-action` are pinned so an XSS cannot rewrite the base
        // URL or post forms cross-origin.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self'; \
                 style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; \
                 font-src 'self' data:; \
                 connect-src 'self'; \
                 frame-ancestors 'none'; \
                 base-uri 'self'; \
                 form-action 'self'; \
                 object-src 'none'",
            ),
        ))
        // Strict-Transport-Security: 2-year preload-eligible policy. Even
        // though the production deployment sits behind Cloudflare (which
        // can manage HSTS itself), emitting this from the origin is
        // defense-in-depth — if someone runs the gateway without CF in
        // front, the policy still gets advertised. Browsers ignore the
        // header on plain-HTTP responses, so it costs nothing on dev.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        // TM-FRONT.8: X-Frame-Options DENY. Belt-and-suspenders alongside
        // the CSP `frame-ancestors 'none'` above; older clients honour the
        // legacy header even when CSP support is partial.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        // TM-FRONT.9: X-Content-Type-Options nosniff blocks MIME-sniffing,
        // so an asset that ends up with a wrong Content-Type can never be
        // executed as a different MIME (script / HTML).
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        // TM-FRONT.7: Permissions-Policy disables geolocation, microphone,
        // and camera. The site has no use for any of them; an XSS therefore
        // cannot prompt the user for permission either.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ))
        // Hide the server name. Tower-http's default trace gives us all the
        // diagnostics we need; advertising "axum" to clients adds nothing.
        // TM-SECRET.4: hidden server header — never advertise framework or
        // version to scanners; the literal "claude-phone" is fixed so a
        // refactor cannot regress it to `axum/0.7.x` or similar.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("server"),
            HeaderValue::from_static("claude-phone"),
        ));

    // TM-RATE.1 — per-IP HTTP cap. burst=10 absorbs reconnect storms on
    // flaky mobile networks; sustained PER_IP_REQ_PER_SEC blocks WS-flood
    // attackers. GovernorLayer is added INSIDE the security-headers layer
    // so 429 responses still carry the strict CSP / HSTS / X-Frame headers
    // (defense in depth on error pages). `/healthz` is intentionally also
    // rate-limited — a hostile flood there is the same DoS vector as on
    // any other route, and a legitimate health probe loop is well under
    // 5/s anyway.
    //
    // Why `.expect`: GovernorConfigBuilder validates on `.finish()`. With
    // hard-coded constants the only failure mode is "constants both zero",
    // which a build-time-visible review (and the forward-looking integration
    // test `per_ip_governor_returns_429_under_burst`) keeps us off of.
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(PER_IP_REQ_PER_SEC)
            .burst_size(PER_IP_BURST)
            .finish()
            // TM-CODE.3: governor builder failure is unreachable for the
            // hard-coded constants above — both are non-zero by definition.
            .expect("GovernorConfigBuilder accepts non-zero per_second + burst_size"),
    );

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
        .layer(GovernorLayer {
            config: governor_conf,
        })
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
