use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::fs;

#[derive(Clone)]
pub struct StaticsState {
    pub dir: PathBuf,
}

/// Serve the React app shell for `/s/<token>` paths.
pub async fn session_shell(
    Path(_token): Path<String>,
    State(state): State<StaticsState>,
) -> Response {
    let index = state.dir.join("index.html");
    serve_file(&index).await
}

pub async fn root(State(state): State<StaticsState>) -> Response {
    let index = state.dir.join("index.html");
    serve_file(&index).await
}

async fn serve_file(path: &std::path::Path) -> Response {
    match fs::read(path).await {
        Ok(bytes) => {
            let content_type = guess_content_type(path);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                bytes,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn guess_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
