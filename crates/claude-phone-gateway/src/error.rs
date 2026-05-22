use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("session not found")]
    SessionNotFound,
    #[error("invalid token")]
    InvalidToken,
    #[error("invalid api key")]
    InvalidApiKey,
    #[error("session already taken")]
    SessionTaken,
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            Self::SessionNotFound | Self::InvalidToken => (StatusCode::NOT_FOUND, self.to_string()),
            Self::InvalidApiKey => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::SessionTaken => (StatusCode::CONFLICT, self.to_string()),
            Self::Internal(_) | Self::Io(_) => {
                tracing::error!(error = ?self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, body).into_response()
    }
}
