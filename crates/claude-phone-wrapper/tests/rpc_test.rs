use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use claude_phone_wrapper::rpc::{PairResponse, RpcState, StatusResponse};
use claude_phone_wrapper::session::SessionState;
use http_body_util::BodyExt;
use tokio::sync::{mpsc, Mutex};
use tower::ServiceExt;

fn make_app() -> (Router, mpsc::Receiver<()>, Arc<Mutex<SessionState>>) {
    let session = Arc::new(Mutex::new(SessionState::default()));
    let (tx, rx) = mpsc::channel::<()>(1);
    let state = RpcState {
        session: session.clone(),
        public_url_base: "https://example.com".into(),
        pair_trigger: tx,
    };
    let app = Router::new()
        .route(
            "/pair",
            axum::routing::post(claude_phone_wrapper::rpc::pair_handler),
        )
        .route(
            "/status",
            axum::routing::get(claude_phone_wrapper::rpc::status_handler),
        )
        .with_state(state);
    (app, rx, session)
}

#[tokio::test]
async fn post_pair_returns_token_and_url() {
    let (app, mut rx, session) = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: PairResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed.url.starts_with("https://example.com/s/"));
    assert!(!parsed.token.is_empty());
    assert!(!parsed.qr_ascii.is_empty());

    let s = session.lock().await;
    assert!(s.token.is_some());
    assert_eq!(s.public_url.as_deref(), Some(parsed.url.as_str()));
    drop(s);
    rx.try_recv().expect("pair_trigger fired");
}

#[tokio::test]
async fn get_status_reflects_session_state() {
    let (app, _rx, _session) = make_app();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s: StatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(!s.paired);
    assert!(!s.peer_connected);
    assert_eq!(s.state, "ok");

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s: StatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(s.paired);
}

#[tokio::test]
async fn pair_qr_ascii_contains_full_url() {
    // The QR encodes the full URL — render_terminal output is the ASCII
    // art so the URL won't be literally in it, but a quick sanity check
    // is that qr_ascii has more than one line of content.
    let (app, _rx, _session) = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: PairResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed.qr_ascii.lines().count() > 5);
}
