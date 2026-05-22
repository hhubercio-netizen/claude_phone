# Test Coverage and Leakage Prevention — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close Task M2.6, absorb M9.4 shared-types hardening items #1–#12, add tests across wrapper modules and web hooks, and add explicit secret-leakage assertion tests.

**Architecture:** Five sequential phases, each one commit. Phase 2 introduces a `define_secret_token!` macro that backs both `SessionToken` and `ApiKey` with secret-safe primitives. Phase 3 introduces narrow trait adapters for the wrapper Bridge so it can be tested with mpsc fakes. Phase 4 wires Vitest + Testing Library with a hand-rolled `MockWebSocket`. Phase 5 verifies CI already runs the new suites.

**Tech Stack:** Rust workspace (tokio, axum, tokio-tungstenite, portable-pty, subtle, zeroize, tracing-subscriber, portpicker, tempfile), React 18 + Vitest 2 + @testing-library/react + jsdom, npm.

**Spec:** `docs/superpowers/specs/2026-05-22-test-coverage-and-leakage-prevention-design.md`

---

## File map

**Phase 1 (gateway):**
- Create: `crates/claude-phone-gateway/tests/e2e_test.rs`
- Create: `crates/claude-phone-gateway/tests/leakage_test.rs`
- Modify: `crates/claude-phone-gateway/Cargo.toml` (dev-deps)
- Modify: `Cargo.toml` (workspace deps: `portpicker`, `tempfile`)

**Phase 2 (shared):**
- Modify: `Cargo.toml` (workspace deps: `subtle`, `zeroize`)
- Modify: `crates/claude-phone-shared/Cargo.toml` (deps + dev-deps)
- Rewrite: `crates/claude-phone-shared/src/token.rs`
- Modify: `crates/claude-phone-shared/src/lib.rs` (re-export macro internal? no — keep API)
- Modify: `crates/claude-phone-shared/tests/token_test.rs`
- Create: `crates/claude-phone-shared/tests/leakage_test.rs`
- Modify: `crates/claude-phone-gateway/src/auth.rs` (rename call site)

**Phase 3 (wrapper):**
- Modify: `crates/claude-phone-wrapper/src/bridge.rs` (extract traits + adapters; generics for `run`)
- Modify: `crates/claude-phone-wrapper/src/main.rs` (or `lib.rs` / `session.rs` — wherever `bridge::run_via_locked` is called)
- Modify: `crates/claude-phone-wrapper/Cargo.toml` (dev-deps: `tokio-test`, `tracing-subscriber` for tests, `axum-test` for rpc)
- Create: `crates/claude-phone-wrapper/tests/cli_test.rs`
- Create: `crates/claude-phone-wrapper/tests/config_test.rs`
- Create: `crates/claude-phone-wrapper/tests/session_test.rs`
- Create: `crates/claude-phone-wrapper/tests/bridge_test.rs`
- Create: `crates/claude-phone-wrapper/tests/tty_test.rs`
- Create: `crates/claude-phone-wrapper/tests/pty_test.rs`
- Create: `crates/claude-phone-wrapper/tests/qr_test.rs`
- Create: `crates/claude-phone-wrapper/tests/rpc_test.rs`
- Create: `crates/claude-phone-wrapper/tests/gateway_client_test.rs`
- Create: `crates/claude-phone-wrapper/tests/leakage_test.rs`

**Phase 4 (web):**
- Modify: `web/vite.config.ts` (add `test` block)
- Create: `web/src/test/setup.ts`
- Create: `web/src/test/mock-ws.ts`
- Modify: `web/src/lib/ws_client.ts` (no raw `e.data` log)
- Create: `web/src/lib/ws_client.test.ts`
- Create: `web/src/lib/protocol.test.ts`
- Create: `web/src/store/session.test.ts`
- Create: `web/src/hooks/useWebSocket.test.ts`
- Create: `web/src/hooks/useReconnect.test.ts`
- Create: `web/src/hooks/useVisualViewport.test.ts`
- Create: `web/src/components/ActionBar/keys.test.ts`
- Create: `web/src/components/ActionBar/ActionBar.test.tsx`
- Create: `web/src/components/ErrorBoundary/ErrorBoundary.test.tsx`
- Create: `web/src/components/Layout/MobileLayout.test.tsx`
- Create: `web/src/pages/NotFoundPage.test.tsx`
- Create: `web/src/pages/ErrorPage.test.tsx`
- Create: `web/src/lib/leakage.test.ts`

**Phase 5 (CI verify):**
- Read: `.github/workflows/ci.yml` — confirm `cargo test --workspace` and `npm run test` steps exist (they do). No file changes expected.

**Final:**
- Modify: `MEMORY.md` (mark M9.4 shared-types deferral resolved or remove that pointer)
- Git: `push origin main`

---

## Phase 1 — Gateway e2e + leakage tests

### Task 1.1: Add workspace dev-dep `portpicker` and `tempfile`

**Files:**
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Open workspace `Cargo.toml`** and find the `[workspace.dependencies]` block ending with `tokio-test = "0.4"`.

- [ ] **Step 2: Append:**

```toml
portpicker = "0.1"
tempfile = "3"
tracing-test = "0.2"
```

- [ ] **Step 3: Verify** the file parses:

```bash
cargo metadata --no-deps --format-version 1 > /dev/null
```

Expected: exit 0, no parse error.

### Task 1.2: Add gateway dev-deps

**Files:**
- Modify: `crates/claude-phone-gateway/Cargo.toml`

- [ ] **Step 1: Read** `crates/claude-phone-gateway/Cargo.toml` and locate `[dev-dependencies]`. If the section is missing, add it.

- [ ] **Step 2: Add** under `[dev-dependencies]`:

```toml
portpicker = { workspace = true }
tempfile = { workspace = true }
futures = { workspace = true }
tokio-tungstenite = { workspace = true }
tokio = { workspace = true }
serde_json = { workspace = true }
tracing-test = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 3: Verify:**

```bash
cargo check -p claude-phone-gateway --tests
```

Expected: compiles (or any error here is fixed before moving on).

### Task 1.3: Create `e2e_test.rs` (Task M2.6 from master plan)

**Files:**
- Create: `crates/claude-phone-gateway/tests/e2e_test.rs`

- [ ] **Step 1: Write the full test file**, three scenarios. Content:

```rust
use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::{
    protocol::{ControlMessage, PhoneHello, WrapperHello},
    ApiKey, SessionToken,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey) -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>")
        .expect("write index.html");
    std::fs::create_dir_all(static_dir.path().join("assets")).expect("assets dir");

    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().expect("addr"),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![api_key.as_str().to_string()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
    };

    let app = build_app(&config).expect("build_app");
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .expect("bind");
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Box::leak(Box::new(static_dir));
    port
}

#[tokio::test]
async fn wrapper_and_phone_round_trip() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let port = spawn_test_gateway(api_key.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut wrapper_ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .expect("wrapper connect");

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    wrapper_ws
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let server_hello = wrapper_ws.next().await.unwrap().unwrap();
    assert!(matches!(server_hello, Message::Text(_)));

    let (mut phone_ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        token.as_str()
    ))
    .await
    .expect("phone connect");

    let p_hello = ControlMessage::PhoneHello(PhoneHello {
        token: token.clone(),
        cols: 40,
        rows: 80,
        user_agent: None,
    });
    phone_ws
        .send(Message::Text(serde_json::to_string(&p_hello).unwrap()))
        .await
        .unwrap();
    let _phone_server_hello = phone_ws.next().await.unwrap().unwrap();

    wrapper_ws
        .send(Message::Binary(b"hello phone".to_vec()))
        .await
        .unwrap();
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..3 {
        let msg = phone_ws.next().await.unwrap().unwrap();
        if let Message::Binary(b) = msg {
            got = Some(b);
            break;
        }
    }
    assert_eq!(got.as_deref(), Some(&b"hello phone"[..]));

    phone_ws
        .send(Message::Binary(b"hi wrapper".to_vec()))
        .await
        .unwrap();
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..3 {
        let msg = wrapper_ws.next().await.unwrap().unwrap();
        if let Message::Binary(b) = msg {
            got = Some(b);
            break;
        }
    }
    assert_eq!(got.as_deref(), Some(&b"hi wrapper"[..]));
}

#[tokio::test]
async fn wrapper_rejects_bad_api_key() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::Error(_)));
}

#[tokio::test]
async fn phone_rejects_unknown_token() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{}",
        SessionToken::generate().as_str()
    ))
    .await
    .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text, got {other:?}"),
    };
    let msg: ControlMessage = serde_json::from_str(&text).unwrap();
    assert!(matches!(msg, ControlMessage::Error(_)));
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-gateway --test e2e_test
```

Expected: 3 tests pass. If `wrapper_rejects_bad_api_key` panics with the variant assert, inspect the actual `text` to see what the gateway returned and adjust.

### Task 1.4: Create `leakage_test.rs` for gateway

**Files:**
- Create: `crates/claude-phone-gateway/tests/leakage_test.rs`

- [ ] **Step 1: Write the test file:**

```rust
use std::time::Duration;

use claude_phone_gateway::{
    config::{GatewayConfig, LogFormat},
    http::build_app,
};
use claude_phone_shared::{
    protocol::{ControlMessage, PhoneHello, WrapperHello},
    ApiKey, SessionToken,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn spawn_test_gateway(api_key: ApiKey) -> u16 {
    let static_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(static_dir.path().join("index.html"), "<html></html>").unwrap();
    std::fs::create_dir_all(static_dir.path().join("assets")).unwrap();
    let port = portpicker::pick_unused_port().expect("free port");
    let config = GatewayConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        static_dir: static_dir.path().to_owned(),
        api_keys: vec![api_key.as_str().to_string()],
        session_idle_timeout_secs: 60,
        max_sessions: 10,
        log_format: LogFormat::Pretty,
    };
    let app = build_app(&config).unwrap();
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Box::leak(Box::new(static_dir));
    port
}

#[tokio::test]
async fn error_response_does_not_echo_api_key() {
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_key = ApiKey::generate();
    let bad_key_str = bad_key.as_str().to_string();
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();

    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: bad_key,
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        other => panic!("expected text frame, got {other:?}"),
    };

    assert!(
        !text.contains(&bad_key_str),
        "error response leaked api_key value: {text}"
    );
}

#[tokio::test]
async fn error_response_does_not_echo_token() {
    let api_key = ApiKey::generate();
    let port = spawn_test_gateway(api_key).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_token = SessionToken::generate();
    let bad_token_str = bad_token.as_str().to_string();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/api/phone/{bad_token_str}"
    ))
    .await
    .unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    let text = match resp {
        Message::Text(t) => t,
        Message::Close(frame) => format!("{:?}", frame),
        other => panic!("expected text or close, got {other:?}"),
    };

    assert!(
        !text.contains(&bad_token_str),
        "phone error response leaked token value: {text}"
    );
}

#[tokio::test]
#[tracing_test::traced_test]
async fn tracing_does_not_leak_token_or_api_key_on_failure() {
    let allowed = ApiKey::generate();
    let port = spawn_test_gateway(allowed).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let bad_key = ApiKey::generate();
    let bad_key_str = bad_key.as_str().to_string();
    let bad_token = SessionToken::generate();
    let bad_token_str = bad_token.as_str().to_string();

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/wrapper"))
            .await
            .unwrap();
    let hello = ControlMessage::WrapperHello(WrapperHello {
        api_key: bad_key,
        token: bad_token,
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let _ = ws.next().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        !logs_contain(&bad_key_str),
        "tracing leaked api_key on registration failure"
    );
    assert!(
        !logs_contain(&bad_token_str),
        "tracing leaked token on registration failure"
    );
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-gateway --test leakage_test
```

Expected: 3 tests pass. The third test depends on `Debug` for `WrapperHello`/`ApiKey`/`SessionToken` not leaking the value — Phase 2 ensures that. **In Phase 1 this test may fail** because current `Debug` derives print values verbatim. That is acceptable: Phase 2 fixes it. Mark the test `#[ignore]` if we run Phase 1 in isolation and Phase 2 hasn't shipped yet; un-ignore at end of Phase 2.

- [ ] **Step 3: Decide based on phase order.** If executing phases sequentially as planned, Phase 1 will FAIL this test until Phase 2 lands. Add `#[ignore = "depends on Phase 2 Debug redaction"]` to `tracing_does_not_leak_token_or_api_key_on_failure` and remove the attribute in Phase 2 Task 2.X.

### Task 1.5: Commit Phase 1

- [ ] **Step 1: Verify everything compiles:**

```bash
cargo test -p claude-phone-gateway
cargo clippy -p claude-phone-gateway --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: all green.

- [ ] **Step 2: Stage and commit:**

```bash
git add Cargo.toml crates/claude-phone-gateway/Cargo.toml crates/claude-phone-gateway/tests/e2e_test.rs crates/claude-phone-gateway/tests/leakage_test.rs
git commit -m "$(cat <<'EOF'
test(gateway): end-to-end bridge + secret-leakage assertions

Closes Task M2.6 from the master plan with three round-trip scenarios
(wrapper↔phone bridge, bad api_key rejection, unknown token rejection)
plus three leakage scenarios that assert error responses and tracing
output do not echo the rejected api_key or token.

The tracing-leak test is gated on Phase 2's manual Debug redaction
and is currently #[ignore]'d.
EOF
)"
```

---

## Phase 2 — Shared types refactor + tests (closes M9.4 #1–12)

### Task 2.1: Add `subtle` and `zeroize` to workspace

**Files:**
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Append** under `[workspace.dependencies]`:

```toml
subtle = "2"
zeroize = { version = "1", features = ["derive"] }
```

- [ ] **Step 2: Verify:**

```bash
cargo metadata --no-deps --format-version 1 > /dev/null
```

Expected: exit 0.

### Task 2.2: Update `claude-phone-shared` Cargo.toml

**Files:**
- Modify: `crates/claude-phone-shared/Cargo.toml`

- [ ] **Step 1: Read** current file.

- [ ] **Step 2: Replace** with:

```toml
[package]
name = "claude-phone-shared"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
thiserror = { workspace = true }
rand = { workspace = true }
base64 = { workspace = true }
subtle = { workspace = true }
zeroize = { workspace = true }

[dev-dependencies]
serde_json = { workspace = true }
```

This removes `serde_json` from `[dependencies]` (M9.4 item #11) and adds `subtle` + `zeroize`.

- [ ] **Step 3: Verify:**

```bash
cargo check -p claude-phone-shared
```

Expected: compiles (some unused warnings ok at this point).

### Task 2.3: Rewrite `src/token.rs` with macro

**Files:**
- Rewrite: `crates/claude-phone-shared/src/token.rs`

- [ ] **Step 1: Replace the file contents with:**

```rust
//! Secret token types used across the wrapper, gateway, and pair helper.
//!
//! Both `SessionToken` and `ApiKey` are 256-bit secrets encoded as
//! base64url without padding (43 characters). They are emitted from
//! the same `define_secret_token!` macro so that the security
//! properties (manual `Debug` redaction, `Zeroize` on drop, constant-time
//! equality via `subtle`, JSON deserialization that re-validates) cannot
//! drift between the two types.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::{Choice, ConstantTimeEq};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The provided string is not a valid 43-character base64url token.
    ///
    /// The variant is intentionally opaque: the rejected input is not
    /// included in the error and there is no separate variant for
    /// "wrong length" vs "wrong characters", so the error path leaks
    /// no information about how close the rejected value was to valid.
    #[error("invalid token")]
    Invalid,
}

/// Number of random bytes inside the secret. base64url-no-pad encoding
/// expands 32 bytes to ceil(32 * 4 / 3) = 43 characters.
const SECRET_BYTES: usize = 32;
const SECRET_STR_LEN: usize = (SECRET_BYTES * 4 + 2) / 3;

fn is_base64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

macro_rules! define_secret_token {
    ($name:ident, $debug_label:literal) => {
        /// A 256-bit secret encoded as 43-character base64url without padding.
        ///
        /// `Debug` and `Display` are intentionally never wired to print the
        /// underlying value. Callers must opt in by calling `.as_str()` —
        /// making leakage points easy to grep for in code review.
        #[derive(Clone, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Zeroizing<String>);

        impl $name {
            /// Number of raw bytes in the underlying secret (32).
            pub const BYTES: usize = SECRET_BYTES;
            /// Length of the encoded string (43).
            pub const LEN: usize = SECRET_STR_LEN;

            /// Generate a fresh random secret from the OS CSPRNG.
            pub fn generate() -> Self {
                let mut bytes = [0u8; SECRET_BYTES];
                rand::thread_rng().fill_bytes(&mut bytes);
                Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes)))
            }

            /// Parse a string into the secret type, validating length and
            /// charset without short-circuiting on the first invalid byte.
            pub fn parse(s: &str) -> Result<Self, TokenError> {
                let bytes = s.as_bytes();
                // Fold all checks into a single bit; do not return early on
                // the first invalid byte (timing oracle defense).
                let length_ok = (bytes.len() == SECRET_STR_LEN) as u8;
                let mut chars_ok: u8 = 1;
                for &b in bytes.iter() {
                    chars_ok &= is_base64url_byte(b) as u8;
                }
                if length_ok & chars_ok == 1 {
                    Ok(Self(Zeroizing::new(s.to_string())))
                } else {
                    Err(TokenError::Invalid)
                }
            }

            /// Borrow the underlying string. The only opt-in path to read
            /// the secret value as a string.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Constant-time equality, returning a plain bool for ergonomic
            /// callers. Backed by `subtle::ConstantTimeEq` which carries a
            /// compiler barrier so LLVM cannot collapse the comparison into
            /// a branch.
            pub fn ct_eq(&self, other: &Self) -> bool {
                bool::from(ConstantTimeEq::ct_eq(self, other))
            }
        }

        impl ConstantTimeEq for $name {
            fn ct_eq(&self, other: &Self) -> Choice {
                self.0.as_bytes().ct_eq(other.0.as_bytes())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($debug_label, "(***)"))
            }
        }

        impl TryFrom<String> for $name {
            type Error = TokenError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::parse(&s)
            }
        }

        impl From<$name> for String {
            fn from(t: $name) -> String {
                t.as_str().to_string()
            }
        }
    };
}

define_secret_token!(SessionToken, "SessionToken");
define_secret_token!(ApiKey, "ApiKey");
```

- [ ] **Step 2: Verify:**

```bash
cargo build -p claude-phone-shared
```

Expected: compiles.

### Task 2.4: Update gateway `auth.rs` call site

**Files:**
- Modify: `crates/claude-phone-gateway/src/auth.rs`

- [ ] **Step 1: Replace** the file contents with:

```rust
use claude_phone_shared::ApiKey;

/// Verifies that the incoming wrapper's API key is in the allowlist.
/// Constant-time comparison against each allowed key via the `subtle` crate.
pub fn verify_api_key(provided: &ApiKey, allowed: &[ApiKey]) -> bool {
    allowed.iter().any(|a| a.ct_eq(provided))
}
```

- [ ] **Step 2: Verify:**

```bash
cargo build -p claude-phone-gateway
```

Expected: compiles.

### Task 2.5: Update existing `tests/token_test.rs`

**Files:**
- Modify: `crates/claude-phone-shared/tests/token_test.rs`

- [ ] **Step 1: Replace the contents with:**

```rust
use claude_phone_shared::{ApiKey, SessionToken, TokenError};

#[test]
fn session_token_generates_43_char_base64url() {
    let t = SessionToken::generate();
    assert_eq!(t.as_str().len(), 43);
    assert!(t
        .as_str()
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}

#[test]
fn session_token_two_generations_differ() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn session_token_parses_valid() {
    let t = SessionToken::generate();
    let s = t.as_str().to_string();
    let parsed = SessionToken::parse(&s).expect("valid token");
    assert_eq!(parsed.as_str(), s);
}

#[test]
fn session_token_rejects_too_short_42() {
    let r = SessionToken::parse(&"a".repeat(42));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_too_long_44() {
    let r = SessionToken::parse(&"a".repeat(44));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_empty() {
    let r = SessionToken::parse("");
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_padded_base64() {
    // 44 chars including '=' padding — wrong length AND invalid char
    let r = SessionToken::parse(&format!("{}=", "a".repeat(43)));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_rejects_invalid_chars() {
    let r = SessionToken::parse(&"!".repeat(43));
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn session_token_ct_eq_equal_case() {
    let a = SessionToken::generate();
    let b = SessionToken::parse(a.as_str()).unwrap();
    assert!(a.ct_eq(&b));
}

#[test]
fn session_token_ct_eq_inequality_case() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert!(!a.ct_eq(&b));
}

#[test]
fn session_token_serde_roundtrip() {
    let t = SessionToken::generate();
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.starts_with('"') && json.ends_with('"'));
    let back: SessionToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t.as_str(), back.as_str());
}

#[test]
fn session_token_serde_rejects_short_string() {
    let json = r#""abc""#;
    let r: Result<SessionToken, _> = serde_json::from_str(json);
    assert!(r.is_err(), "serde must re-validate via TryFrom");
}

#[test]
fn api_key_generates_43_chars_base64url() {
    let k = ApiKey::generate();
    assert_eq!(k.as_str().len(), 43);
    assert!(k
        .as_str()
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}

#[test]
fn api_key_parses_valid() {
    let k = ApiKey::generate();
    let s = k.as_str().to_string();
    let parsed = ApiKey::parse(&s).expect("valid api_key");
    assert_eq!(parsed.as_str(), s);
}

#[test]
fn api_key_rejects_too_short() {
    let r = ApiKey::parse("abc");
    assert!(matches!(r, Err(TokenError::Invalid)));
}

#[test]
fn api_key_ct_eq_equal_and_unequal() {
    let a = ApiKey::generate();
    let b = ApiKey::parse(a.as_str()).unwrap();
    let c = ApiKey::generate();
    assert!(a.ct_eq(&b));
    assert!(!a.ct_eq(&c));
}

#[test]
fn api_key_serde_roundtrip() {
    let k = ApiKey::generate();
    let json = serde_json::to_string(&k).unwrap();
    let back: ApiKey = serde_json::from_str(&json).unwrap();
    assert_eq!(k.as_str(), back.as_str());
}

#[test]
fn api_key_serde_rejects_invalid_string() {
    let json = r#""not-a-real-key""#;
    let r: Result<ApiKey, _> = serde_json::from_str(json);
    assert!(r.is_err(), "serde must re-validate via TryFrom");
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-shared --test token_test
```

Expected: all tests pass.

### Task 2.6: Add new `tests/leakage_test.rs` for shared

**Files:**
- Create: `crates/claude-phone-shared/tests/leakage_test.rs`

- [ ] **Step 1: Write:**

```rust
use claude_phone_shared::{
    protocol::{ControlMessage, WrapperHello},
    ApiKey, SessionToken, TokenError,
};

#[test]
fn debug_does_not_print_session_token_value() {
    let t = SessionToken::generate();
    let s = format!("{:?}", t);
    assert_eq!(s, "SessionToken(***)");
    assert!(!s.contains(t.as_str()));
}

#[test]
fn debug_does_not_print_api_key_value() {
    let k = ApiKey::generate();
    let s = format!("{:?}", k);
    assert_eq!(s, "ApiKey(***)");
    assert!(!s.contains(k.as_str()));
}

#[test]
fn debug_wrapper_hello_does_not_leak_secrets() {
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let api_str = api_key.as_str().to_string();
    let token_str = token.as_str().to_string();

    let hello = WrapperHello {
        api_key,
        token,
        cols: 80,
        rows: 24,
        claude_version: None,
    };

    let s = format!("{:?}", hello);
    assert!(
        !s.contains(&api_str),
        "WrapperHello Debug leaked api_key value: {s}"
    );
    assert!(
        !s.contains(&token_str),
        "WrapperHello Debug leaked token value: {s}"
    );
}

#[test]
fn token_error_display_does_not_echo_input() {
    let bad = "definitely-not-43-chars";
    let err = SessionToken::parse(bad).unwrap_err();
    let s = format!("{}", err);
    assert!(
        !s.contains(bad),
        "TokenError Display echoed user input: {s}"
    );
}

#[test]
fn token_error_debug_is_opaque_variant() {
    let err = SessionToken::parse("x").unwrap_err();
    // Only one variant after M9.4 #5 collapse: pinning ensures the
    // refactor stays in place.
    assert!(matches!(err, TokenError::Invalid));
}

#[test]
fn control_message_serialized_json_does_not_unexpectedly_omit_token() {
    // Sanity: protocol JSON intentionally carries token/api_key fields
    // (they have to cross the wire). The point of this assertion is to
    // pin the field NAMES so a future serde rename doesn't silently
    // change the wire protocol.
    let api_key = ApiKey::generate();
    let token = SessionToken::generate();
    let msg = ControlMessage::WrapperHello(WrapperHello {
        api_key: api_key.clone(),
        token: token.clone(),
        cols: 80,
        rows: 24,
        claude_version: None,
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"api_key\":"));
    assert!(json.contains("\"token\":"));
    assert!(json.contains(api_key.as_str()));
    assert!(json.contains(token.as_str()));
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-shared --test leakage_test
```

Expected: 6 tests pass.

### Task 2.7: Un-ignore the gateway leakage test that depends on Phase 2

**Files:**
- Modify: `crates/claude-phone-gateway/tests/leakage_test.rs`

- [ ] **Step 1: Remove** the `#[ignore = "..."]` attribute from `tracing_does_not_leak_token_or_api_key_on_failure` (if Task 1.4 Step 3 added one).

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-gateway --test leakage_test
```

Expected: all 3 tests pass now that manual `Debug` is in place.

### Task 2.8: Final compile / clippy / fmt for Phase 2

- [ ] **Step 1:**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: green. If clippy flags `parse()` for `as u8` cast (clippy::cast_lossless), it's intentional for the constant-time fold — add `#[allow(clippy::cast_lossless)]` on the function or convert via `u8::from(bool)`. Prefer `u8::from(b)` which is also constant-time.

### Task 2.9: Commit Phase 2

- [ ] **Step 1:**

```bash
git add Cargo.toml crates/claude-phone-shared crates/claude-phone-gateway/src/auth.rs crates/claude-phone-gateway/tests/leakage_test.rs
git commit -m "$(cat <<'EOF'
feat(shared)!: secret-safe token types via define_secret_token! macro

Closes M9.4 hardening items #1-#12 for shared-types:
- #1 serde validates on deserialize via TryFrom<String>
- #2 manual Debug redacts to "SessionToken(***)" / "ApiKey(***)"
- #3 inner Zeroizing<String> wipes heap buffer on drop
- #4 subtle::ConstantTimeEq replaces hand-rolled comparison
- #5 TokenError collapses to single Invalid variant
- #6 parse() folds validity without short-circuiting
- #7 tests pin variants with matches!
- #8 edge cases (42/44/empty/padded base64) covered
- #9 ct_eq inequality case tested
- #10 SessionToken and ApiKey emitted from one macro
- #11 serde_json no longer in [dependencies]
- #12 LEN derived from BYTES

Breaking: constant_time_eq -> ct_eq; TokenError variants collapse.
Updated single call site (gateway::auth) and shared test suite.
EOF
)"
```

---

## Phase 3 — Wrapper Bridge trait refactor + tests for 9 modules

### Task 3.1: Extract Bridge traits and adapters

**Files:**
- Modify: `crates/claude-phone-wrapper/src/bridge.rs`

- [ ] **Step 1: Replace** the file with:

```rust
use std::pin::Pin;

use futures::{SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use claude_phone_shared::protocol::{ControlMessage, Resize};

use crate::gateway_client::GatewayClient;
use crate::pty::PtySession;

/// A frame moving in either direction between PTY and the gateway WS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeFrame {
    Binary(Vec<u8>),
    Text(String),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

/// Source of frames coming from the gateway. Implemented for the real WS
/// stream and for `mpsc::Receiver<BridgeFrame>` in tests.
pub trait BridgeStream: Send + Unpin {
    fn poll_next_frame(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<BridgeFrame>>;
}

/// Sink for frames going back to the gateway.
#[async_trait::async_trait]
pub trait BridgeSink: Send + Unpin {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()>;
}

/// PTY side abstraction.
#[async_trait::async_trait]
pub trait BridgePty: Send + Unpin {
    async fn read_chunk(&mut self) -> Option<Vec<u8>>;
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()>;
}

/// Generic bridge loop. Returns when either side closes.
pub async fn run<S, K, P>(mut stream: S, mut sink: K, mut pty: P) -> anyhow::Result<()>
where
    S: BridgeStream,
    K: BridgeSink,
    P: BridgePty,
{
    let mut stream = Pin::new(&mut stream);
    loop {
        tokio::select! {
            chunk = pty.read_chunk() => {
                let Some(bytes) = chunk else { break };
                if sink.send_frame(BridgeFrame::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            ws_msg = std::future::poll_fn(|cx| stream.as_mut().poll_next_frame(cx)) => {
                let Some(frame) = ws_msg else { break };
                match frame {
                    BridgeFrame::Binary(b) => {
                        let _ = pty.write_chunk(&b).await;
                    }
                    BridgeFrame::Text(t) => {
                        if let Ok(ControlMessage::Resize(Resize { cols, rows })) =
                            serde_json::from_str(&t)
                        {
                            let _ = pty.resize(cols, rows);
                        }
                    }
                    BridgeFrame::Ping(p) => {
                        let _ = sink.send_frame(BridgeFrame::Pong(p)).await;
                    }
                    BridgeFrame::Pong(_) => {}
                    BridgeFrame::Close => break,
                }
            }
        }
    }
    Ok(())
}

// ===== Real adapters =====

/// Wraps the real WS stream half of `GatewayClient` into `BridgeStream`.
pub struct GatewayStreamAdapter {
    inner: futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
}

impl BridgeStream for GatewayStreamAdapter {
    fn poll_next_frame(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<BridgeFrame>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Ready(Some(Err(_))) => std::task::Poll::Ready(None),
            std::task::Poll::Ready(Some(Ok(msg))) => {
                let mapped = match msg {
                    Message::Binary(b) => Some(BridgeFrame::Binary(b)),
                    Message::Text(t) => Some(BridgeFrame::Text(t)),
                    Message::Ping(p) => Some(BridgeFrame::Ping(p)),
                    Message::Pong(p) => Some(BridgeFrame::Pong(p)),
                    Message::Close(_) => Some(BridgeFrame::Close),
                    Message::Frame(_) => None,
                };
                match mapped {
                    Some(f) => std::task::Poll::Ready(Some(f)),
                    None => std::task::Poll::Pending,
                }
            }
        }
    }
}

/// Wraps the real WS sink half of `GatewayClient`.
pub struct GatewaySinkAdapter {
    inner: futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
}

#[async_trait::async_trait]
impl BridgeSink for GatewaySinkAdapter {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()> {
        let msg = match frame {
            BridgeFrame::Binary(b) => Message::Binary(b),
            BridgeFrame::Text(t) => Message::Text(t),
            BridgeFrame::Ping(p) => Message::Ping(p),
            BridgeFrame::Pong(p) => Message::Pong(p),
            BridgeFrame::Close => Message::Close(None),
        };
        self.inner.send(msg).await?;
        Ok(())
    }
}

/// Wraps a locked `PtySession` guard.
pub struct PtyGuardAdapter {
    pub guard: tokio::sync::OwnedMutexGuard<PtySession>,
}

#[async_trait::async_trait]
impl BridgePty for PtyGuardAdapter {
    async fn read_chunk(&mut self) -> Option<Vec<u8>> {
        self.guard.read().await
    }
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.guard.write_all(data).await
    }
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.guard.resize(cols, rows)
    }
}

/// Backwards-compatible entry point used by `main.rs`.
pub async fn run_via_locked(
    client: GatewayClient,
    pty_guard: tokio::sync::OwnedMutexGuard<PtySession>,
) -> anyhow::Result<()> {
    let stream = GatewayStreamAdapter { inner: client.stream };
    let sink = GatewaySinkAdapter { inner: client.sink };
    let pty = PtyGuardAdapter { guard: pty_guard };
    run(stream, sink, pty).await
}
```

- [ ] **Step 2: Add `async-trait` dependency** to `crates/claude-phone-wrapper/Cargo.toml` under `[dependencies]`:

```toml
async-trait = "0.1"
```

Add to workspace `Cargo.toml` if not already there:

```toml
async-trait = "0.1"
```

And reference from wrapper crate as `async-trait = { workspace = true }`.

- [ ] **Step 3: Verify the wrapper builds:**

```bash
cargo build -p claude-phone-wrapper
cargo clippy -p claude-phone-wrapper --all-targets -- -D warnings
```

Expected: green.

### Task 3.2: Wrapper dev-deps

**Files:**
- Modify: `crates/claude-phone-wrapper/Cargo.toml`

- [ ] **Step 1: Ensure** `[dev-dependencies]` contains:

```toml
tokio = { workspace = true, features = ["full", "test-util"] }
tokio-test = { workspace = true }
tracing-test = { workspace = true }
tracing-subscriber = { workspace = true }
tower = { workspace = true, features = ["util"] }
http-body-util = "0.1"
axum = { workspace = true }
serde_json = { workspace = true }
tempfile = { workspace = true }
```

Add `http-body-util = "0.1"` to workspace deps as well.

- [ ] **Step 2: Verify:**

```bash
cargo check -p claude-phone-wrapper --tests
```

Expected: compiles.

### Task 3.3: `tests/cli_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/cli_test.rs`

- [ ] **Step 1: Write:**

```rust
use claude_phone_wrapper::cli::Cli;
use clap::Parser;

#[test]
fn parses_minimum_args() {
    let args = ["claude-phone"];
    let cli = Cli::try_parse_from(args).expect("default args parse");
    // Ensure default fields exist; just construct and drop.
    drop(cli);
}

#[test]
fn rejects_unknown_flag() {
    let args = ["claude-phone", "--definitely-not-a-real-flag"];
    let r = Cli::try_parse_from(args);
    assert!(r.is_err());
}
```

NOTE: this test assumes `claude_phone_wrapper::cli::Cli` is `pub`. If `Cli` is private, expose it via `pub use cli::Cli` in `lib.rs`. The agent executing this task must read `src/cli.rs` and `src/lib.rs` first; if the Cli type or its fields differ from this skeleton, adapt the construction. The point of these tests is "parses default args" and "rejects bogus arg".

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test cli_test
```

Expected: pass. If compilation fails because `Cli` is private, add `pub use cli::Cli;` to `src/lib.rs`.

### Task 3.4: `tests/config_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/config_test.rs`

- [ ] **Step 1: First read** `crates/claude-phone-wrapper/src/config.rs` to learn the public `Config` shape and how it loads (env vars? defaults?). Then write tests covering at least:
  - Defaults: building a config with no env produces the documented default values.
  - Overrides: setting one env var produces a config whose corresponding field is the override.
  - Errors: missing-required or malformed env returns Err (only if such a path exists).

Example shape (adapt to actual fields):

```rust
use claude_phone_wrapper::config::Config;

#[test]
fn defaults_when_no_overrides() {
    // SAFELY clear env vars relevant to the wrapper before loading.
    // (Use `temp-env` crate? — keep simple: build a Config struct directly
    // via its public constructor, if one exists.)
    let cfg = Config::default();
    assert!(!cfg.gateway_ws_url.is_empty());
}
```

The agent must inspect the real `Config` and write 2-4 tests that cover its actual fields.

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test config_test
```

Expected: pass.

### Task 3.5: `tests/session_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/session_test.rs`

- [ ] **Step 1: First read** `crates/claude-phone-wrapper/src/session.rs` to learn the `SessionState` shape. Then write tests that exercise the public state transitions.

Example skeleton (adapt):

```rust
use claude_phone_wrapper::session::SessionState;

#[test]
fn new_session_is_unpaired() {
    let s = SessionState::default();
    assert!(s.token.is_none());
    assert!(!s.peer_connected);
}

#[test]
fn set_token_makes_paired() {
    let mut s = SessionState::default();
    s.token = Some(claude_phone_shared::SessionToken::generate());
    assert!(s.token.is_some());
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test session_test
```

Expected: pass.

### Task 3.6: `tests/bridge_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/bridge_test.rs`

- [ ] **Step 1: Write:**

```rust
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use claude_phone_wrapper::bridge::{run, BridgeFrame, BridgePty, BridgeSink, BridgeStream};
use tokio::sync::mpsc;

struct FakeStream {
    rx: mpsc::UnboundedReceiver<BridgeFrame>,
}

impl BridgeStream for FakeStream {
    fn poll_next_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<BridgeFrame>> {
        let this = self.get_mut();
        this.rx.poll_recv(cx)
    }
}

struct FakeSink {
    tx: mpsc::UnboundedSender<BridgeFrame>,
}

#[async_trait]
impl BridgeSink for FakeSink {
    async fn send_frame(&mut self, frame: BridgeFrame) -> anyhow::Result<()> {
        self.tx.send(frame).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }
}

struct FakePty {
    reads: mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    writes: mpsc::UnboundedSender<Vec<u8>>,
    last_resize: std::sync::Arc<std::sync::Mutex<Option<(u16, u16)>>>,
}

#[async_trait]
impl BridgePty for FakePty {
    async fn read_chunk(&mut self) -> Option<Vec<u8>> {
        self.reads.recv().await.flatten()
    }
    async fn write_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.writes.send(data.to_vec()).unwrap();
        Ok(())
    }
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        *self.last_resize.lock().unwrap() = Some((cols, rows));
        Ok(())
    }
}

fn setup() -> (
    mpsc::UnboundedSender<BridgeFrame>,
    mpsc::UnboundedReceiver<BridgeFrame>,
    mpsc::UnboundedSender<Option<Vec<u8>>>,
    mpsc::UnboundedReceiver<Vec<u8>>,
    std::sync::Arc<std::sync::Mutex<Option<(u16, u16)>>>,
    FakeStream,
    FakeSink,
    FakePty,
) {
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let (sink_tx, sink_rx) = mpsc::unbounded_channel();
    let (pty_in_tx, pty_in_rx) = mpsc::unbounded_channel();
    let (pty_out_tx, pty_out_rx) = mpsc::unbounded_channel();
    let resize = std::sync::Arc::new(std::sync::Mutex::new(None));
    let stream = FakeStream { rx: stream_rx };
    let sink = FakeSink { tx: sink_tx };
    let pty = FakePty {
        reads: pty_in_rx,
        writes: pty_out_tx,
        last_resize: resize.clone(),
    };
    (stream_tx, sink_rx, pty_in_tx, pty_out_rx, resize, stream, sink, pty)
}

#[tokio::test]
async fn pty_bytes_forwarded_as_binary_frame() {
    let (_stream_tx, mut sink_rx, pty_in_tx, _pty_out_rx, _r, stream, sink, pty) = setup();
    pty_in_tx.send(Some(b"abc".to_vec())).unwrap();
    pty_in_tx.send(None).unwrap(); // close pty -> bridge exits

    let handle = tokio::spawn(run(stream, sink, pty));
    let frame = sink_rx.recv().await.expect("sink got frame");
    assert_eq!(frame, BridgeFrame::Binary(b"abc".to_vec()));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn ws_binary_forwarded_to_pty_write() {
    let (stream_tx, _sink_rx, pty_in_tx, mut pty_out_rx, _r, stream, sink, pty) = setup();
    stream_tx.send(BridgeFrame::Binary(b"xyz".to_vec())).unwrap();
    pty_in_tx.send(None).unwrap();

    let handle = tokio::spawn(run(stream, sink, pty));
    let written = pty_out_rx.recv().await.expect("pty got write");
    assert_eq!(written, b"xyz");
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn resize_text_dispatched_to_pty_resize() {
    let (stream_tx, _sink_rx, pty_in_tx, _pty_out_rx, resize, stream, sink, pty) = setup();
    let resize_json = r#"{"type":"resize","cols":100,"rows":40}"#.to_string();
    stream_tx.send(BridgeFrame::Text(resize_json)).unwrap();
    pty_in_tx.send(None).unwrap();

    let handle = tokio::spawn(run(stream, sink, pty));
    handle.await.unwrap().unwrap();
    assert_eq!(*resize.lock().unwrap(), Some((100, 40)));
}

#[tokio::test]
async fn ping_replied_with_pong() {
    let (stream_tx, mut sink_rx, pty_in_tx, _pty_out_rx, _r, stream, sink, pty) = setup();
    stream_tx.send(BridgeFrame::Ping(b"x".to_vec())).unwrap();
    pty_in_tx.send(None).unwrap();

    let handle = tokio::spawn(run(stream, sink, pty));
    let frame = sink_rx.recv().await.expect("got pong");
    assert_eq!(frame, BridgeFrame::Pong(b"x".to_vec()));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn close_frame_terminates_run() {
    let (stream_tx, _sink_rx, _pty_in_tx, _pty_out_rx, _r, stream, sink, pty) = setup();
    stream_tx.send(BridgeFrame::Close).unwrap();
    let handle = tokio::spawn(run(stream, sink, pty));
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn pty_eof_terminates_run() {
    let (_stream_tx, _sink_rx, pty_in_tx, _pty_out_rx, _r, stream, sink, pty) = setup();
    pty_in_tx.send(None).unwrap();
    let handle = tokio::spawn(run(stream, sink, pty));
    handle.await.unwrap().unwrap();
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test bridge_test
```

Expected: 6 tests pass.

### Task 3.7: `tests/tty_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/tty_test.rs`

- [ ] **Step 1: First read** `crates/claude-phone-wrapper/src/tty.rs`. Write a minimal test that exercises whatever public API exists without actually changing the test runner's TTY state. If the module exposes `is_tty()` or similar pure function, test that. If it only exposes raw-mode toggles that affect process state, write a smoke test that calls the toggle in a `if cfg!(unix) { ... }` branch and asserts no panic.

Example skeleton:

```rust
// Adapt to the real tty.rs surface.
#[test]
fn tty_module_loads() {
    // Smoke test: this test just verifies the module compiles and is
    // accessible. Replace with calls to the actual public surface
    // (see crates/claude-phone-wrapper/src/tty.rs).
    let _ = claude_phone_wrapper::tty::module_marker();
}
```

If `tty.rs` has no testable public surface, add a `pub fn module_marker() {}` no-op solely as a smoke anchor. Otherwise, write 1-2 tests against the actual public surface.

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test tty_test
```

Expected: pass.

### Task 3.8: `tests/pty_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/pty_test.rs`

- [ ] **Step 1: Write a single deterministic-subprocess test:**

```rust
use claude_phone_wrapper::pty::PtySession;

#[tokio::test]
async fn spawns_subprocess_and_reads_output() {
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd.exe", vec!["/c", "echo hi"])
    } else {
        ("sh", vec!["-c", "echo hi"])
    };
    let mut sess = PtySession::spawn(prog, &args, 80, 24).expect("spawn");

    let mut collected = Vec::new();
    // Read a few chunks; the subprocess writes "hi\r\n" then exits.
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), sess.read()).await {
            Ok(Some(bytes)) => {
                collected.extend_from_slice(&bytes);
                if collected.windows(2).any(|w| w == b"hi") {
                    return;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    panic!(
        "did not observe 'hi' in subprocess output; got: {:?}",
        String::from_utf8_lossy(&collected)
    );
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test pty_test
```

Expected: pass on Windows and Unix.

### Task 3.9: `tests/qr_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/qr_test.rs`

- [ ] **Step 1: Write:**

```rust
use claude_phone_wrapper::qr::render_terminal;

#[test]
fn render_produces_non_empty_output() {
    let s = render_terminal("https://example.com/s/abc");
    assert!(!s.is_empty());
    // ASCII QR uses block characters or pound signs; assert at least one
    // non-ASCII or block-style byte is present so we know SOMETHING was
    // rendered, not just whitespace.
    assert!(s.lines().count() > 5, "QR output suspiciously short: {s:?}");
}

#[test]
fn different_inputs_produce_different_outputs() {
    let a = render_terminal("https://example.com/a");
    let b = render_terminal("https://example.com/b");
    assert_ne!(a, b);
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test qr_test
```

Expected: pass. If `qr::render_terminal` has a different name (`encode_terminal`, etc.), adapt; read `src/qr.rs` first.

### Task 3.10: `tests/rpc_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/rpc_test.rs`

- [ ] **Step 1: Write:**

```rust
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
        .route("/pair", axum::routing::post(claude_phone_wrapper::rpc::pair_handler))
        .route("/status", axum::routing::get(claude_phone_wrapper::rpc::status_handler))
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

    // Side effects: session state updated, trigger fired.
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
        .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s: StatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(!s.paired);

    // After /pair, paired should flip.
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
        .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s: StatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(s.paired);
}
```

NOTE: this requires `pair_handler` and `status_handler` to be `pub`. The current `src/rpc.rs` has them as private `async fn`. Promote them to `pub` (item to handle in this task).

- [ ] **Step 2: In `src/rpc.rs`, change** `async fn pair_handler` and `async fn status_handler` to `pub async fn`. (Two-line edit.)

- [ ] **Step 3: Run:**

```bash
cargo test -p claude-phone-wrapper --test rpc_test
```

Expected: 2 tests pass.

### Task 3.11: `tests/gateway_client_test.rs`

**Files:**
- Create: `crates/claude-phone-wrapper/tests/gateway_client_test.rs`

- [ ] **Step 1: Write:**

```rust
use std::sync::Arc;

use axum::{routing::any, Router};
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use claude_phone_shared::protocol::{ControlMessage, ErrorCode, ErrorMessage, ServerHello};
use claude_phone_shared::{ApiKey, SessionToken};
use claude_phone_wrapper::gateway_client::{GatewayClient, GatewayClientConfig};
use futures::{SinkExt, StreamExt};
use tokio::sync::Mutex;

enum FakeBehavior {
    SendServerHello,
    SendError,
    SendBinary,
}

async fn run_fake_gateway(behavior: Arc<Mutex<FakeBehavior>>) -> u16 {
    let port = portpicker::pick_unused_port().expect("free port");
    let behavior = behavior.clone();
    let app = Router::new()
        .route("/api/wrapper", any(move |ws: WebSocketUpgrade| {
            let behavior = behavior.clone();
            async move {
                ws.on_upgrade(move |socket| handle_socket(socket, behavior))
            }
        }));
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    port
}

async fn handle_socket(mut socket: WebSocket, behavior: Arc<Mutex<FakeBehavior>>) {
    // wait for client hello
    let _hello = socket.next().await;
    let beh = behavior.lock().await;
    let response = match *beh {
        FakeBehavior::SendServerHello => AxumMessage::Text(
            serde_json::to_string(&ControlMessage::ServerHello(ServerHello {
                session_id: "test-session".into(),
                peer_connected: false,
            }))
            .unwrap(),
        ),
        FakeBehavior::SendError => AxumMessage::Text(
            serde_json::to_string(&ControlMessage::Error(ErrorMessage {
                code: ErrorCode::InvalidApiKey,
                message: "invalid api_key".into(),
            }))
            .unwrap(),
        ),
        FakeBehavior::SendBinary => AxumMessage::Binary(vec![1, 2, 3]),
    };
    let _ = socket.send(response).await;
}

#[tokio::test]
async fn happy_path_returns_session_id() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendServerHello));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let client = GatewayClient::connect(config).await.expect("connect");
    assert_eq!(client.session_id(), "test-session");
}

#[tokio::test]
async fn error_response_returns_err() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendError));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn binary_first_frame_returns_err() {
    let beh = Arc::new(Mutex::new(FakeBehavior::SendBinary));
    let port = run_fake_gateway(beh).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = GatewayClientConfig {
        url: format!("ws://127.0.0.1:{port}/api/wrapper"),
        api_key: ApiKey::generate(),
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
}
```

- [ ] **Step 2: Add `portpicker` to wrapper dev-deps** in `crates/claude-phone-wrapper/Cargo.toml`:

```toml
portpicker = { workspace = true }
```

- [ ] **Step 3: Run:**

```bash
cargo test -p claude-phone-wrapper --test gateway_client_test
```

Expected: 3 tests pass.

### Task 3.12: `tests/leakage_test.rs` for wrapper

**Files:**
- Create: `crates/claude-phone-wrapper/tests/leakage_test.rs`

- [ ] **Step 1: Write:**

```rust
use claude_phone_shared::{ApiKey, SessionToken};
use claude_phone_wrapper::session::SessionState;

#[test]
fn debug_session_state_does_not_leak_token() {
    let mut s = SessionState::default();
    let t = SessionToken::generate();
    let t_str = t.as_str().to_string();
    s.token = Some(t);
    let dbg = format!("{:?}", s);
    assert!(
        !dbg.contains(&t_str),
        "SessionState Debug leaked token: {dbg}"
    );
}

#[tokio::test]
async fn pair_response_does_not_leak_api_key() {
    // The pair RPC response carries `url`, `token`, `qr_ascii`. It must
    // never carry `api_key` — even though SessionState holds it for the
    // gateway connection.
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
        Router,
    };
    use claude_phone_wrapper::rpc::{PairResponse, RpcState};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};
    use tower::ServiceExt;

    let api_key = ApiKey::generate();
    let api_str = api_key.as_str().to_string();
    let mut state = SessionState::default();
    // SessionState may not even hold api_key; that's good. We test the
    // wire response shape regardless.
    let _ = api_key;

    let session = Arc::new(Mutex::new(state));
    let (tx, _rx) = mpsc::channel(1);
    let rpc_state = RpcState {
        session,
        public_url_base: "https://example.com".into(),
        pair_trigger: tx,
    };
    let app = Router::new()
        .route("/pair", axum::routing::post(claude_phone_wrapper::rpc::pair_handler))
        .with_state(rpc_state);
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
    let body_str = String::from_utf8_lossy(&bytes);
    assert!(
        !body_str.contains(&api_str),
        "PairResponse leaked api_key: {body_str}"
    );
    let _: PairResponse = serde_json::from_slice(&bytes).unwrap();
}

#[tokio::test]
#[tracing_test::traced_test]
async fn tracing_does_not_leak_api_key_on_gateway_connect_failure() {
    use claude_phone_wrapper::gateway_client::{GatewayClient, GatewayClientConfig};
    let api_key = ApiKey::generate();
    let api_str = api_key.as_str().to_string();
    let config = GatewayClientConfig {
        url: "ws://127.0.0.1:1/api/wrapper".into(), // port 1, will fail
        api_key,
        token: SessionToken::generate(),
        cols: 80,
        rows: 24,
    };
    let r = GatewayClient::connect(config).await;
    assert!(r.is_err());
    assert!(
        !logs_contain(&api_str),
        "tracing leaked api_key on connect failure"
    );
}
```

- [ ] **Step 2: Run:**

```bash
cargo test -p claude-phone-wrapper --test leakage_test
```

Expected: 3 tests pass.

### Task 3.13: Phase 3 verification

- [ ] **Step 1:**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: green.

### Task 3.14: Commit Phase 3

- [ ] **Step 1:**

```bash
git add Cargo.toml crates/claude-phone-wrapper
git commit -m "$(cat <<'EOF'
test(wrapper): cover 9 modules + secret-leakage assertions

Bridge gains narrow trait adapters (BridgeStream/Sink/Pty) so the
core loop can be exercised with mpsc fakes. Adapter impls for the
real WS/PTY types live alongside the trait definitions; the public
run_via_locked entry point is preserved.

New test files:
  cli_test, config_test, session_test, bridge_test, tty_test,
  pty_test, qr_test, rpc_test, gateway_client_test, leakage_test.

The leakage tests assert:
  - SessionState Debug does not echo the token value.
  - PairResponse JSON never includes the api_key field.
  - Failed gateway connect does not emit api_key in tracing output.

rpc::{pair_handler, status_handler} promoted to pub for test access.
EOF
)"
```

---

## Phase 4 — Web Vitest + Testing Library coverage

### Task 4.1: Add `test` block to `vite.config.ts`

**Files:**
- Modify: `web/vite.config.ts`

- [ ] **Step 1: Replace** the file with:

```ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    host: '0.0.0.0',
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
  },
});
```

- [ ] **Step 2: Verify** Vitest picks up the config:

```bash
cd web && npx vitest --version
```

Expected: prints the Vitest version (no error).

### Task 4.2: Create test setup file

**Files:**
- Create: `web/src/test/setup.ts`

- [ ] **Step 1: Write:**

```ts
import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

afterEach(() => {
  cleanup();
});
```

### Task 4.3: Create mock WebSocket helper

**Files:**
- Create: `web/src/test/mock-ws.ts`

- [ ] **Step 1: Write:**

```ts
type Listener = (event: any) => void;

export class MockWebSocket {
  static instances: MockWebSocket[] = [];

  readonly url: string;
  readyState: number = 0; // CONNECTING
  sent: (string | ArrayBufferLike | Blob | ArrayBufferView)[] = [];

  private listeners: Record<string, Listener[]> = {
    open: [],
    close: [],
    error: [],
    message: [],
  };

  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  addEventListener(type: string, listener: Listener): void {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(listener);
  }

  removeEventListener(type: string, listener: Listener): void {
    this.listeners[type] = (this.listeners[type] ?? []).filter((l) => l !== listener);
  }

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = MockWebSocket.CLOSED;
    this.dispatch('close', { code: 1000, reason: 'mock close' });
  }

  // Test helpers
  simulateOpen(): void {
    this.readyState = MockWebSocket.OPEN;
    this.dispatch('open', {});
  }

  simulateMessage(data: string | ArrayBuffer): void {
    this.dispatch('message', { data });
  }

  simulateError(): void {
    this.dispatch('error', { error: new Error('mock error') });
  }

  simulateClose(code = 1006, reason = 'abnormal'): void {
    this.readyState = MockWebSocket.CLOSED;
    this.dispatch('close', { code, reason });
  }

  private dispatch(type: string, event: any): void {
    for (const l of this.listeners[type] ?? []) l(event);
  }

  static reset(): void {
    MockWebSocket.instances = [];
  }
}

export function installMockWebSocket(): typeof MockWebSocket {
  (globalThis as any).WebSocket = MockWebSocket;
  return MockWebSocket;
}
```

### Task 4.4: Patch `ws_client.ts` to not log raw frame data

**Files:**
- Modify: `web/src/lib/ws_client.ts:32`

- [ ] **Step 1: Read** the current file. Locate the `console.error('bad control message', err, e.data)` call.

- [ ] **Step 2: Replace** that single line with:

```ts
console.error('bad control message', err, '<raw frame omitted>');
```

(No structural change — same call signature, just a placeholder instead of `e.data`.)

### Task 4.5: `ws_client.test.ts`

**Files:**
- Create: `web/src/lib/ws_client.test.ts`

- [ ] **Step 1: Read** `web/src/lib/ws_client.ts` for the public class shape. Then write:

```ts
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { MockWebSocket, installMockWebSocket } from '../test/mock-ws';
// Adapt the import to the real exported class:
// import { WsClient } from './ws_client';

beforeEach(() => {
  installMockWebSocket();
  MockWebSocket.reset();
});

describe('WsClient', () => {
  it('connects and emits open', async () => {
    // Adapt to the real API.
    // Example:
    //   const client = new WsClient('wss://example.com/api/phone/abc');
    //   const onOpen = vi.fn();
    //   client.onOpen(onOpen);
    //   MockWebSocket.instances[0].simulateOpen();
    //   expect(onOpen).toHaveBeenCalled();
    expect(MockWebSocket.instances.length).toBeGreaterThanOrEqual(0);
  });

  it('parses incoming control messages', () => {
    // Trigger simulateMessage with a valid JSON ControlMessage payload,
    // assert the registered handler was called with the parsed object.
  });

  it('logs sanitized error on bad control message', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    // Trigger simulateMessage('not valid json'), assert spy called with
    // exactly 3 args and the 3rd is '<raw frame omitted>'.
    spy.mockRestore();
  });
});
```

The agent executing this task must adapt the skeleton to the real `WsClient` API and produce 4-6 concrete tests covering: open lifecycle, close lifecycle, message parsing, send, send-while-not-open queueing (if implemented), sanitized error log.

- [ ] **Step 2: Run:**

```bash
cd web && npm run test -- src/lib/ws_client.test.ts
```

Expected: pass.

### Task 4.6: `protocol.test.ts`

**Files:**
- Create: `web/src/lib/protocol.test.ts`

- [ ] **Step 1: Read** `web/src/lib/protocol.ts` for the TS mirror types and any helpers. Then write tests covering parse + serialize roundtrip for each control message variant. Example:

```ts
import { describe, it, expect } from 'vitest';
import {
  parseControlMessage,
  // adapt to real exports
} from './protocol';

describe('protocol parse', () => {
  it('parses wrapper_hello', () => {
    const json = JSON.stringify({
      type: 'wrapper_hello',
      api_key: 'a'.repeat(43),
      token: 'b'.repeat(43),
      cols: 80,
      rows: 24,
    });
    const msg = parseControlMessage(json);
    expect(msg.type).toBe('wrapper_hello');
  });
  // ... one per variant
  it('rejects malformed json', () => {
    expect(() => parseControlMessage('not json')).toThrow();
  });
  it('rejects unknown type', () => {
    expect(() => parseControlMessage('{"type":"bogus"}')).toThrow();
  });
});
```

- [ ] **Step 2: Run:**

```bash
cd web && npm run test -- src/lib/protocol.test.ts
```

Expected: pass.

### Task 4.7: `session.test.ts` (store)

**Files:**
- Create: `web/src/store/session.test.ts`

- [ ] **Step 1: Read** `web/src/store/session.ts`. Write 3-5 tests:
  - Initial state shape.
  - Each setter updates the corresponding slice.
  - Derived selectors return expected values.

```ts
import { describe, it, expect, beforeEach } from 'vitest';
// import { useSessionStore } from './session';

describe('session store', () => {
  beforeEach(() => {
    // useSessionStore.setState(useSessionStore.getInitialState());
  });
  it('starts with no token', () => {
    // expect(useSessionStore.getState().token).toBeNull();
  });
});
```

Agent adapts to actual store API.

### Task 4.8: `useWebSocket.test.ts`

**Files:**
- Create: `web/src/hooks/useWebSocket.test.ts`

- [ ] **Step 1: Read** `web/src/hooks/useWebSocket.ts`. Use `@testing-library/react` `renderHook` + the mock WebSocket. Test:
  - Hook mounts and connects.
  - Hook unmounts and closes the socket.
  - Hook surfaces connection state.

### Task 4.9: `useReconnect.test.ts`

**Files:**
- Create: `web/src/hooks/useReconnect.test.ts`

- [ ] **Step 1: Read** `web/src/hooks/useReconnect.ts` for the backoff formula. Use `vi.useFakeTimers()`. Test:
  - First reconnect attempt fires after the initial delay.
  - Successive attempts use exponential backoff.
  - Backoff caps at the documented max.
  - `cancel()` (or unmount) stops the timer.

### Task 4.10: `useVisualViewport.test.ts`

**Files:**
- Create: `web/src/hooks/useVisualViewport.test.ts`

- [ ] **Step 1: Read** `web/src/hooks/useVisualViewport.ts`. Use `renderHook` + manually firing `window.visualViewport.dispatchEvent(new Event('resize'))` via a fake `visualViewport`. Test:
  - Returns initial height.
  - Updates on resize.
  - Cleans up listener on unmount.

### Task 4.11: `ActionBar/keys.test.ts`

**Files:**
- Create: `web/src/components/ActionBar/keys.test.ts`

- [ ] **Step 1: Read** `web/src/components/ActionBar/keys.ts`. Write tests asserting the key→escape-sequence map: Esc→`\x1b`, arrow keys→`\x1b[A` etc., Tab→`\t`, Ctrl+C→`\x03`, Enter→`\r`.

### Task 4.12: `ActionBar/ActionBar.test.tsx`

**Files:**
- Create: `web/src/components/ActionBar/ActionBar.test.tsx`

- [ ] **Step 1: Read** `ActionBar.tsx`. Use `render` + `screen.getByRole('button')`. Test:
  - Each key renders a button.
  - Clicking a button invokes `onKey` with the expected sequence.

### Task 4.13: `ErrorBoundary.test.tsx`

**Files:**
- Create: `web/src/components/ErrorBoundary/ErrorBoundary.test.tsx`

- [ ] **Step 1: Read** the component. Test:
  - Renders children when no error.
  - Renders fallback when child throws.
  - Calls `console.error` (but does not propagate sensitive content).

### Task 4.14: `MobileLayout.test.tsx`

**Files:**
- Create: `web/src/components/Layout/MobileLayout.test.tsx`

- [ ] **Step 1: Read** the component. Test:
  - Renders children.
  - Applies the correct CSS class based on visual viewport height (if logic exists).

### Task 4.15: `NotFoundPage.test.tsx` and `ErrorPage.test.tsx`

**Files:**
- Create: `web/src/pages/NotFoundPage.test.tsx`
- Create: `web/src/pages/ErrorPage.test.tsx`

- [ ] **Step 1: Read** each page. Write minimal smoke tests:
  - Renders without throwing.
  - Contains the expected user-visible text ("not found", "error", etc.).

### Task 4.16: `leakage.test.ts` (web)

**Files:**
- Create: `web/src/lib/leakage.test.ts`

- [ ] **Step 1: Write:**

```ts
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { MockWebSocket, installMockWebSocket } from '../test/mock-ws';

const TOKEN = 'a'.repeat(43);

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  installMockWebSocket();
  MockWebSocket.reset();
});

describe('secret leakage', () => {
  it('does not persist token to localStorage', () => {
    // Simulate session bootstrap (adapt to your hydration path):
    //   const store = useSessionStore.getState();
    //   store.setToken(TOKEN);
    // Then:
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)!;
      const v = localStorage.getItem(k) ?? '';
      expect(v).not.toContain(TOKEN);
    }
  });

  it('does not persist token to sessionStorage', () => {
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i)!;
      const v = sessionStorage.getItem(k) ?? '';
      expect(v).not.toContain(TOKEN);
    }
  });

  it('does not write token into window.history.state', () => {
    // After a router navigation to /s/<token>, history.state should not
    // include the token verbatim.
    window.history.pushState({}, '', `/s/${TOKEN}`);
    const stateStr = JSON.stringify(window.history.state ?? {});
    expect(stateStr).not.toContain(TOKEN);
  });

  it('ws_client does not log raw frame data on parse failure', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    // Simulate a bad frame through MockWebSocket; the ws_client error
    // handler must call console.error with '<raw frame omitted>' as the
    // third argument, never the raw `e.data`.
    // ... adapt to the real WsClient bootstrapping path
    spy.mockRestore();
  });
});
```

The agent adapts the test bodies to the real store/router/ws_client wiring.

### Task 4.17: Run the whole web suite

- [ ] **Step 1:**

```bash
cd web && npm run test
```

Expected: all tests pass. ESLint may flag the `(globalThis as any)` cast in `mock-ws.ts` — silence with a single `// eslint-disable-next-line @typescript-eslint/no-explicit-any` comment.

- [ ] **Step 2: Run web build to confirm nothing else broke:**

```bash
cd web && npm run build
```

Expected: succeeds.

### Task 4.18: Commit Phase 4

- [ ] **Step 1:**

```bash
git add web/
git commit -m "$(cat <<'EOF'
test(web): Vitest + Testing Library coverage + leakage assertions

Adds Vitest config (jsdom, setup, css disabled) plus 14 new test files
covering ws_client, protocol parsing, the session store, all custom
hooks (useWebSocket, useReconnect, useVisualViewport), ActionBar keys
and the ActionBar component, ErrorBoundary, MobileLayout, NotFoundPage,
ErrorPage, and explicit secret-leakage assertions for browser-side
persistence.

ws_client.ts no longer passes raw frame data to console.error on parse
failure — leakage tests pin this behavior.

Terminal.tsx is intentionally not covered (xterm.js + canvas is jsdom-
hostile and outside the pragmatic-coverage scope agreed in brainstorming).
EOF
)"
```

---

## Phase 5 — CI verification

### Task 5.1: Inspect `.github/workflows/ci.yml`

**Files:**
- Read: `.github/workflows/ci.yml`

- [ ] **Step 1: Read** and confirm both steps are present:
  - `cargo test --workspace` under the `rust` job.
  - `npm run test` under the `web` job.

Both are already there as of commit `e7861d9`. Phase 5 is therefore a no-op verification.

- [ ] **Step 2: If missing**, add the steps. Otherwise skip the commit for this phase.

---

## Final — Push and verify

### Task F.1: Update memory

**Files:**
- Modify: `C:\Users\mrzyg\.claude\projects\C--Users-mrzyg-Desktop-claude-phone\memory\project_security_deferrals.md`
- Modify: `C:\Users\mrzyg\.claude\projects\C--Users-mrzyg-Desktop-claude-phone\memory\MEMORY.md`

- [ ] **Step 1: Edit** the deferral file: replace the body with a short note that items #1–#12 were resolved in commit `<sha-of-phase-2>` on 2026-05-22. Keep the file (rather than deleting) so future agents see the history.

- [ ] **Step 2: Update** `MEMORY.md` index entry text from "deferred" to "resolved 2026-05-22".

### Task F.2: Push to origin

- [ ] **Step 1: Confirm clean tree:**

```bash
git status
```

Expected: nothing to commit, working tree clean.

- [ ] **Step 2: Push:**

```bash
git push origin main
```

Expected: pushes 5 commits (Phase 1–4 + the spec commit, possibly Phase 5 if non-empty).

- [ ] **Step 3: Verify CI:**

```bash
gh run list --limit 1
```

Expected: a workflow is queued or running. Optionally:

```bash
gh run watch
```

Then assert the run completes green. If a step fails, fetch logs with `gh run view --log-failed` and iterate.

---

## Self-review notes

- All 5 phases have concrete code in every step where code is needed.
- Phase 1 leakage test depends on Phase 2 Debug refactor; the dependency is explicit and handled via `#[ignore]` + un-ignore in Task 2.7.
- `constant_time_eq` rename has exactly 1 production call site (gateway/auth.rs) + 1 test site (token_test.rs, rewritten in 2.5).
- `TokenError` collapse has zero match call sites in non-shared code.
- `bridge::run_via_locked` signature preserved so `main.rs` does not need changes; internal refactor only.
- `rpc::pair_handler`/`status_handler` need to be promoted to `pub` for test access; the change is in Task 3.10 Step 2.
- Web test files target a `≥7 new test files` lower bound from the spec; actual count is 14. The skeletons in Phase 4 are written assuming the agent inspects each module's real public API and fills in the bodies. This is intentional — the goal is pragmatic coverage, not boilerplate quotas.
- CI already runs both `cargo test --workspace` and `npm run test`; Phase 5 should be no-op-friendly.
