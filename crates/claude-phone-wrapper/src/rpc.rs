use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use claude_phone_shared::ApiKey;

use crate::qr;
use crate::session::SessionState;

#[derive(Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub url: String,
    pub token: String,
    pub qr_ascii: String,
}

/// Manual Debug that redacts `token`, the token-bearing `url`, and the
/// `qr_ascii` block (the QR encodes the token URL — its bytes ARE the secret).
/// Without this, `tracing::debug!(?response, ...)` anywhere on the wrapper
/// side would write the bearer-equivalent token to wrapper.log.
impl std::fmt::Debug for PairResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairResponse")
            .field("url", &"<redacted>")
            .field("token", &"<redacted>")
            .field("qr_ascii", &format!("<{} bytes>", self.qr_ascii.len()))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: String,
    pub paired: bool,
    pub peer_connected: bool,
}

/// Shared state for the wrapper's local RPC server.
///
/// `auth_token` is an ephemeral 256-bit ApiKey generated each time the
/// wrapper starts. It is propagated to the spawned `claude` child via the
/// `CLAUDE_PHONE_RPC_TOKEN` environment variable so the `claude-phone-pair`
/// helper invoked from inside `claude` can authenticate. The bind defaults
/// to `127.0.0.1`, but binding alone is not enough on a multi-user box: a
/// browser at 127.0.0.1, a co-tenant, or a rogue VS Code extension can all
/// reach the loopback. The bearer is what makes this safe.
#[derive(Clone)]
pub struct RpcState {
    pub session: Arc<Mutex<SessionState>>,
    pub public_url_base: String,
    pub pair_trigger: mpsc::Sender<()>,
    pub auth_token: ApiKey,
}

pub struct RpcServer {
    pub local_addr: SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl RpcServer {
    pub async fn start_with_state(bind: &str, state: RpcState) -> anyhow::Result<Self> {
        let app = build_router(state);
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let local_addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        Ok(Self {
            local_addr,
            _handle: handle,
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.local_addr)
    }
}

/// Build the RPC router with bearer-auth applied to every route.
///
/// Exposed (vs. inlined in `start_with_state`) so unit tests can drive the
/// router via `tower::ServiceExt::oneshot` and exercise the auth path.
pub fn build_router(state: RpcState) -> Router {
    Router::new()
        .route("/pair", post(pair_handler))
        .route("/status", get(status_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// Rejects requests that don't carry a valid `Authorization: Bearer <token>`
/// header. The bearer is compared constant-time against the ephemeral token
/// the wrapper generated at startup.
async fn auth_middleware(
    State(state): State<RpcState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let raw = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(bearer) = raw.and_then(|h| h.strip_prefix("Bearer ")) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    // ApiKey::parse re-validates length and charset without short-circuit.
    // The validation timing depends only on the public protocol shape
    // (length/charset), not on the secret value.
    let provided = ApiKey::parse(bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if !state.auth_token.ct_eq(&provided) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

pub async fn pair_handler(State(state): State<RpcState>) -> Json<PairResponse> {
    use claude_phone_shared::SessionToken;
    let token = SessionToken::generate();
    let url = format!("{}/s/{}", state.public_url_base, token.as_str());
    let qr_ascii = qr::render_terminal(&url);
    {
        let mut s = state.session.lock().await;
        s.token = Some(token.clone());
        s.public_url = Some(url.clone());
    }
    let _ = state.pair_trigger.try_send(());
    Json(PairResponse {
        url,
        token: token.as_str().to_string(),
        qr_ascii,
    })
}

pub async fn status_handler(State(state): State<RpcState>) -> Json<StatusResponse> {
    let s = state.session.lock().await;
    Json(StatusResponse {
        state: "ok".into(),
        paired: s.token.is_some(),
        peer_connected: s.peer_connected,
    })
}
