// TM-SECRET.11 — opaque error chain in gateway responses.
//
// Every variant of `GatewayError` must produce a 4xx/5xx body that contains
// only a static, category-level string. No `anyhow` chain, no `io::Error`
// message, no internal path or query, no token, no api_key. The server-side
// log carries the full diagnostic via `tracing::error!`; the wire body is
// deliberately bland so an unauthenticated probe cannot fingerprint the
// gateway's internals (database vendor, file paths, dependency stack
// traces) by triggering errors and reading the response.
//
// These tests are forward-looking. They will fail if:
//   - a future variant gains a `{0}` interpolation that exposes internal
//     state in the Display impl,
//   - the `IntoResponse` mapping starts forwarding `self.to_string()` for
//     `Internal` / `Io` (right now only the static 4xx variants do),
//   - the literal body strings are renamed without updating this catalog,
//   - a regression switches the body to JSON with an `error.cause` field.
//
// The reference axum 0.7 utility `body::to_bytes(body, limit)` reads the
// full Response body in one shot; 1024 is several orders of magnitude
// larger than any legitimate error body and traps a regression that
// accidentally streams a stack trace.

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use claude_phone_gateway::error::GatewayError;

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1024)
        .await
        .expect("body reads within 1KB cap");
    String::from_utf8(bytes.to_vec()).expect("error bodies are valid utf-8")
}

#[tokio::test]
async fn internal_variant_body_is_literal_internal_error() {
    // The Internal variant wraps an anyhow chain. A buggy IntoResponse that
    // wrote `self.to_string()` would forward the chain to the wire and leak
    // (here) a fake DB URL with a password and an internal hostname.
    let secret = "postgres://root:hunter2@db.internal:5432/prod";
    let err = GatewayError::Internal(anyhow::anyhow!("db connect failed: {secret}"));
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let text = body_text(resp).await;
    assert_eq!(text, "internal error", "5xx body must be the literal");
    assert!(!text.contains("postgres://"), "scheme must not leak");
    assert!(!text.contains("hunter2"), "password must not leak");
    assert!(
        !text.contains("db.internal"),
        "internal hostname must not leak"
    );
}

#[tokio::test]
async fn internal_variant_with_anyhow_context_is_still_opaque() {
    // anyhow's `.context()` builder produces multi-line Display output. A
    // regression that calls `format!("{:#}", err)` on the wire would dump
    // the entire chain. Guard against the most common shape.
    let err = GatewayError::Internal(
        anyhow::anyhow!("root cause: /etc/claude-phone/gateway.toml")
            .context("loading config")
            .context("startup"),
    );
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let text = body_text(resp).await;
    assert_eq!(text, "internal error");
    assert!(!text.contains("/etc/claude-phone"), "fs path must not leak");
    assert!(
        !text.contains("loading config"),
        "context layer must not leak"
    );
}

#[tokio::test]
async fn io_variant_body_is_literal_internal_error() {
    // io::Error Display typically includes the OS-level message ("Permission
    // denied", "No such file or directory") AND the path that triggered it.
    // Both are fingerprintable; neither should ride the response.
    let secret_path = "/var/lib/claude-phone/sessions/SECRET-FILE";
    let io_err = std::io::Error::other(format!("permission denied on {secret_path}"));
    let err: GatewayError = io_err.into();
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let text = body_text(resp).await;
    assert_eq!(text, "internal error");
    assert!(!text.contains("permission denied"));
    assert!(!text.contains("SECRET-FILE"));
    assert!(!text.contains("/var/lib"));
}

#[tokio::test]
async fn session_not_found_body_is_static_category() {
    // 404 path. Body is the thiserror Display string verbatim. Pin it so
    // a "let's add the requested path to the body for debugging" PR fails
    // the test instead of leaking the token-shaped value.
    let resp = GatewayError::SessionNotFound.into_response();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(resp).await, "session not found");
}

#[tokio::test]
async fn invalid_token_body_is_static_category() {
    // 404 path. Same shape as SessionNotFound — distinct text so a client
    // can disambiguate "you used a malformed token" vs "your token is
    // syntactically fine but unknown server-side", but BOTH map to 404 so
    // the wire response doesn't reveal which case it is. Pin the text so a
    // future "let's tell them WHICH character was bad" patch fails here.
    let resp = GatewayError::InvalidToken.into_response();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(resp).await, "invalid token");
}

#[tokio::test]
async fn invalid_api_key_body_is_static_category() {
    // 401 path. Body MUST NOT echo the key (the rejected value), the list
    // of accepted keys, or any "did you mean" hint.
    let resp = GatewayError::InvalidApiKey.into_response();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_text(resp).await, "invalid api key");
}

#[tokio::test]
async fn session_taken_body_is_static_category() {
    // 409 path. Pinning the body so a future "session was taken at <time>
    // by <peer ip>" enrichment fails the test — that data is for the
    // server-side log, not the unauthenticated wire response.
    let resp = GatewayError::SessionTaken.into_response();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(body_text(resp).await, "session already taken");
}

#[tokio::test]
async fn five_hundred_body_size_is_bounded() {
    // Defense-in-depth: even a `to_bytes(.., 1024)` cap would mask a
    // body that grew unbounded with the anyhow chain. Re-assert the body
    // is short (a fixed literal). 64 chars is generous for "internal
    // error" and any plausible literal a future maintainer would write.
    let err = GatewayError::Internal(anyhow::anyhow!(
        "{}",
        "X".repeat(50_000) // explicit large payload — must NOT make the wire
    ));
    let resp = err.into_response();
    let text = body_text(resp).await;
    assert!(
        text.len() <= 64,
        "internal-error body is {} bytes; should be a fixed literal",
        text.len()
    );
}

#[tokio::test]
async fn five_hundred_body_does_not_contain_secret_shaped_substring() {
    // Generic shape probe: a future maintainer might keep the body short
    // but unintentionally include an opaque-looking hash that's actually a
    // session token or api_key. Scan the body for any 43-char base64url
    // window and reject. This mirrors the leakage_test.rs Debug check.
    fn has_token_shape(s: &str) -> bool {
        const LEN: usize = 43;
        if s.len() < LEN {
            return false;
        }
        s.as_bytes().windows(LEN).any(|w| {
            w.iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    }
    let err = GatewayError::Internal(anyhow::anyhow!("synthetic"));
    let resp = err.into_response();
    let text = body_text(resp).await;
    assert!(
        !has_token_shape(&text),
        "5xx body contains a 43-char base64url window — possible secret leak: {text}"
    );
}
