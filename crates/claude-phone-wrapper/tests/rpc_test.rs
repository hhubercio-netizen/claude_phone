use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use claude_phone_shared::ApiKey;
use claude_phone_wrapper::rpc::{build_router, PairResponse, RpcState, StatusResponse};
use claude_phone_wrapper::session::SessionState;
use http_body_util::BodyExt;
use tokio::sync::{mpsc, Mutex};
use tower::ServiceExt;

struct Harness {
    app: Router,
    auth: ApiKey,
    pair_rx: mpsc::Receiver<()>,
    session: Arc<Mutex<SessionState>>,
}

fn make_harness() -> Harness {
    let session = Arc::new(Mutex::new(SessionState::default()));
    let (tx, rx) = mpsc::channel::<()>(1);
    let auth = ApiKey::generate();
    let state = RpcState {
        session: session.clone(),
        public_url_base: "https://example.com".into(),
        pair_trigger: tx,
        auth_token: auth.clone(),
    };
    Harness {
        app: build_router(state),
        auth,
        pair_rx: rx,
        session,
    }
}

fn bearer(token: &ApiKey) -> String {
    format!("Bearer {}", token.as_str())
}

#[tokio::test]
async fn post_pair_returns_token_and_url() {
    let mut h = make_harness();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, bearer(&h.auth))
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

    let s = h.session.lock().await;
    assert!(s.token.is_some());
    assert_eq!(s.public_url.as_deref(), Some(parsed.url.as_str()));
    drop(s);
    h.pair_rx.try_recv().expect("pair_trigger fired");
}

#[tokio::test]
async fn get_status_reflects_session_state() {
    let h = make_harness();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/status")
                .header(header::AUTHORIZATION, bearer(&h.auth))
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

    let _ = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, bearer(&h.auth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/status")
                .header(header::AUTHORIZATION, bearer(&h.auth))
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
async fn pair_qr_ascii_contains_multiple_lines() {
    let h = make_harness();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, bearer(&h.auth))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed: PairResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed.qr_ascii.lines().count() > 5);
}

#[tokio::test]
async fn pair_without_authorization_rejected_401() {
    let h = make_harness();
    let resp = h
        .app
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
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn status_without_authorization_rejected_401() {
    let h = make_harness();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pair_with_wrong_bearer_rejected_401() {
    let h = make_harness();
    let wrong = ApiKey::generate();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, bearer(&wrong))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pair_with_malformed_bearer_rejected_401() {
    let h = make_harness();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pair_with_basic_auth_rejected_401() {
    // The middleware insists on the "Bearer " scheme. Anything else (Basic,
    // ApiKey, Digest, or naked token) must be rejected so we never silently
    // accept a non-Bearer header that happens to contain the right bytes.
    let h = make_harness();
    let raw = format!("Basic {}", h.auth.as_str());
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/pair")
                .header(header::AUTHORIZATION, raw)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
