use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::qr;
use crate::session::SessionState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub url: String,
    pub token: String,
    pub qr_ascii: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: String,
    pub paired: bool,
    pub peer_connected: bool,
}

#[derive(Clone)]
pub struct RpcState {
    pub session: Arc<Mutex<SessionState>>,
    pub public_url_base: String,
    pub pair_trigger: mpsc::Sender<()>,
}

pub struct RpcServer {
    pub local_addr: SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl RpcServer {
    pub async fn start_with_state(bind: &str, state: RpcState) -> anyhow::Result<Self> {
        let app = Router::new()
            .route("/pair", post(pair_handler))
            .route("/status", get(status_handler))
            .with_state(state);

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
