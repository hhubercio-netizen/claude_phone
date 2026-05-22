# Test Coverage and Secret-Leakage Prevention — Design

**Status:** Approved (brainstorming session 2026-05-22)
**Owner:** main branch, direct commits, push to origin on completion
**Implementation:** autonomous (Opus 4.7 max effort)

---

## 1. Context and motivation

Claude Phone v1 has shipped end-to-end functionality across wrapper, gateway, web and plugin (commits up to `c12f546`). Two gaps remain before the project meets the "enterprise-grade" bar the user set at project inception:

1. **Test coverage** — Several modules have no tests at all (wrapper, web). Gateway has unit tests for `config`/`registry`/`auth` but the planned end-to-end bridge test (Task M2.6) was never written. Shared types have basic tests but miss the edge cases flagged in the M1.1 code review.
2. **Secret-leakage exposure** — `SessionToken` and `ApiKey` derive `Debug`, which prints the secret value verbatim. Any `tracing::warn!(error = ?e, …)` whose error chain captures a token leaks it to logs. Web `console.error('bad control message', err, e.data)` leaks raw JSON (potentially containing tokens) to the browser console. No tests currently assert that these surfaces stay clean.

This spec defines a focused push that closes both gaps in one coordinated effort. It also absorbs the M9.4 deferred-hardening items (1–12 from `project_security_deferrals.md`) related to shared-types security, because they are the same code that the leakage tests will exercise.

---

## 2. Goals and non-goals

### Goals

- Close Task M2.6 from the master plan (gateway e2e bridge test).
- Add tests to every untested module in `crates/claude-phone-wrapper` (cli, config, session, bridge, tty, pty, qr, rpc, gateway_client).
- Add Vitest + Testing Library tests to `web/` for ws_client, hooks (useWebSocket, useReconnect, useVisualViewport), session store, ActionBar key mapping, ErrorBoundary, MobileLayout, NotFoundPage, ErrorPage.
- Add token/api-key edge case coverage in `crates/claude-phone-shared` (length 42/44, empty, padded base64, ApiKey serde roundtrip, protocol roundtrip).
- Close M9.4 shared-types hardening items #1–#12 listed in `project_security_deferrals.md`.
- Add explicit secret-leakage assertion tests on 4 surfaces: shared `Debug`/`Display`/error messages, gateway tracing + WS error responses, wrapper tracing + stderr, web localStorage/sessionStorage/history/console.
- Add a CI step that runs the new test suites.

### Non-goals

- Playwright / real-browser e2e (we explicitly chose Vitest with `Terminal.tsx` skipped — xterm.js needs canvas which jsdom lacks).
- Coverage tooling (`cargo tarpaulin`, `vitest --coverage`) — we chose pragmatic over chasing percentages.
- Cross-language protocol fixture (Rust JSON serialized → TS parses identically). Each side gets its own roundtrip test, no shared fixture.
- Live smoke test on `claude-phone.pl`. Stays as a separate effort.
- Other M9.4 hardening items not in items #1–#12 (e.g., rate limiting, audit log).

### Success criteria

- `cargo test --workspace` green; gateway has the new `e2e_test.rs`, shared has new edge-case and leakage tests, wrapper has tests for all 9 modules.
- `cd web && pnpm test` green with ≥7 new test files.
- `cargo clippy --workspace -- -D warnings` and `cargo fmt --check` still pass.
- M3 manual smoke test from `docs/development.md` still passes after the wrapper refactor — no behavior regression.
- A `tracing::Layer` capture-based test demonstrates: a wrapper-registration failure path emits zero log lines containing the rejected token or api-key.
- CI workflow has a `test` job for both Rust and web that runs to completion.
- `git push origin main` succeeds (28 prior commits + this push's commits all reach origin).

---

## 3. Test architecture per area

### 3.1 Gateway (`crates/claude-phone-gateway/`)

**Files to add:**
- `tests/e2e_test.rs` — Task M2.6 from the master plan. The plan has the full Rust code for three scenarios:
  - `wrapper_and_phone_round_trip`: wrapper connects, phone connects, both directions binary frames bridge correctly.
  - `wrapper_rejects_bad_api_key`: WS responds with `ControlMessage::Error` for unknown api-key.
  - `phone_rejects_unknown_token`: WS responds with `ControlMessage::Error` for unknown token.
- `tests/leakage_test.rs` — three additional scenarios:
  - `error_response_does_not_echo_api_key`: invalid wrapper hello, assert `ErrorMessage.message` does not contain the api-key value.
  - `error_response_does_not_echo_token`: invalid phone hello, assert `ErrorMessage.message` does not contain the token value.
  - `tracing_does_not_leak_token_on_registration_failure`: use a `tracing_subscriber::layer::SubscriberExt` test layer that captures formatted lines; trigger registration failure (e.g., duplicate token); assert no captured line contains the token string.

**Dev-deps to add (workspace level):**
- `portpicker = "0.1"` (already referenced in plan M2.6)
- `tempfile = "3"` (already in workspace deps)
- `tracing-subscriber` for `EnvFilter` and test capture (likely already present; verify)

### 3.2 Shared (`crates/claude-phone-shared/`)

**Refactor (closes M9.4 items #1–#12):**

Replace the contents of `src/token.rs` with a single `define_secret_token!` macro (item #10) that emits both `SessionToken` and `ApiKey` from the same definition. The emitted type has:

- `const BYTES: usize = 32` and `const LEN: usize = (BYTES * 4 + 2) / 3` (item #12 — derived, not hardcoded).
- `#[serde(try_from = "String", into = "String")]` instead of `#[serde(transparent)]` (item #1). `TryFrom<String>` calls `parse()` so JSON deserialization enforces the invariant.
- Manual `Debug` impl returning `"SessionToken(***)"` / `"ApiKey(***)"` (item #2).
- `Display` is **not** implemented for either type; callers must explicitly `.as_str()` to print, making leakage points greppable.
- The inner field is `zeroize::Zeroizing<String>` (item #3). Its own `Drop` impl zeros the heap buffer before deallocation; no extra `Drop`/`ZeroizeOnDrop` derive is needed on the outer type. `Clone` is implemented manually to keep the inner type as `Zeroizing<String>`.
- `parse()` uses a non-short-circuiting fold over the bytes, accumulating a single validity bit (item #6) — no early return on first invalid char.
- `TokenError` collapsed to one variant: `Invalid` (item #5). Display message: `"invalid token"`.
- The hand-rolled `constant_time_eq` is removed. Item #4: implement `subtle::ConstantTimeEq` for both types (delegates to `subtle::Choice` math, which carries a compiler barrier). Plus an inherent shortcut `pub fn ct_eq(&self, other: &Self) -> bool` that calls the trait and converts via `Choice::into()` — call sites stay readable. `Eq`/`PartialEq` not derived on these types — comparison only via `ct_eq` to prevent accidental timing-leaky `==`.

**New test file:** `tests/leakage_test.rs`:
- `debug_does_not_print_token_value`: `format!("{:?}", token).contains(token.as_str())` is false.
- `debug_does_not_print_api_key_value`: same for `ApiKey`.
- `wrapper_hello_debug_does_not_leak_secrets`: derived `Debug` on `WrapperHello` delegates to field `Debug` — assert it does not contain either value.
- `error_message_does_not_quote_input`: `TokenError::Invalid` `Display` does not include the rejected input.

**Extended test file:** `tests/token_test.rs` (update existing):
- Add `length_42_rejected`, `length_44_rejected`, `empty_rejected`, `padded_base64_rejected` (item #8).
- Pin variant: `matches!(r, Err(TokenError::Invalid))` (item #7).
- `constant_time_eq_inequality_case` for both types (item #9).
- `ApiKey::parse` parity tests (item #8).
- `roundtrip_serde_session_token` and `roundtrip_serde_api_key`: `serde_json::from_str` / `to_string` using the new `try_from`/`into` path (item #1 verification).

**Extended test file:** `tests/protocol_test.rs` (update existing):
- Roundtrip every `ControlMessage` variant (WrapperHello, PhoneHello, ServerHello, Error, Resize, PeerStatus, Close).
- `wrapper_hello_with_unknown_field_fails`: future-proof — `#[serde(deny_unknown_fields)]` may not be set, but we capture today's behavior.

**Cargo.toml changes:**
- `serde_json` removed from `[dependencies]` (item #11). It stays in `[dev-dependencies]`.
- Add `zeroize = { version = "1", features = ["derive"] }` and `subtle = "2"` to workspace `Cargo.toml`.

### 3.3 Wrapper (`crates/claude-phone-wrapper/`)

**Light refactor for testability (chosen approach — option 1 from brainstorming):**

The Bridge module currently takes `GatewayClient` and `OwnedMutexGuard<PtySession>` by value, both concrete. Tests cannot exercise it without a real PTY and real WS. Two narrow trait pairs unblock isolated tests:

```rust
// crates/claude-phone-wrapper/src/bridge.rs
pub trait BridgeStream: Send {
    async fn next(&mut self) -> Option<Result<BridgeFrame>>;
}

pub trait BridgeSink: Send {
    async fn send(&mut self, frame: BridgeFrame) -> Result<()>;
}

pub trait BridgePty: Send {
    async fn read(&mut self) -> Option<Vec<u8>>;
    async fn write_all(&mut self, data: &[u8]) -> Result<()>;
    fn resize(&self, cols: u16, rows: u16) -> Result<()>;
}

pub enum BridgeFrame {
    Binary(Vec<u8>),
    Text(String),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}
```

`bridge::run_via_locked` becomes `bridge::run<S, K, P>` taking generics. Existing call sites in `main.rs` wrap `GatewayClient.stream`, `GatewayClient.sink`, and `OwnedMutexGuard<PtySession>` in adapter impls. Adapters live in `bridge.rs` to keep the trait surface co-located.

This refactor must produce zero behavior change. Verification: `cargo test`, `cargo clippy`, and `docs/development.md` smoke test all still pass.

**Files to add:**

- `tests/cli_test.rs` — argument parsing via clap `try_parse_from` (happy path + invalid arg).
- `tests/config_test.rs` — env override of defaults; missing required envs return error.
- `tests/session_test.rs` — `SessionState` transitions: `Unpaired → Paired (token set) → PeerConnected → Disconnected`.
- `tests/bridge_test.rs` — using fake `BridgeStream`/`BridgeSink`/`BridgePty` backed by `mpsc::channel`. Scenarios:
  - PTY-to-WS: stream PTY bytes, assert sink receives `BridgeFrame::Binary`.
  - WS-to-PTY: feed `BridgeFrame::Binary`, assert PTY write buffer matches.
  - Resize: feed `Text(json_resize)`, assert PTY resize called with cols/rows.
  - Ping → Pong: feed `Ping(b"x")`, assert sink received `Pong(b"x")`.
  - Close terminates: feed `Close`, assert run returns Ok.
  - PTY EOF terminates: drop PTY tx, assert run returns Ok.
- `tests/tty_test.rs` — raw-mode entry/exit toggles state. (TTY interaction is OS-dependent; test the small state wrapper, not the underlying ioctls. If wrapper logic is a single `if cfg!(unix)` branch, smoke-test that the function exists and returns Ok in CI's headless environment.)
- `tests/pty_test.rs` — spawn a deterministic subprocess (`cmd /c echo hi` on Windows, `echo hi` on Unix); read until EOF; assert "hi" appears in the collected output. One test only — this is real-PTY territory, we accept the platform branch.
- `tests/qr_test.rs` — `render_terminal(url)` produces non-empty output containing expected QR-block characters; different URLs produce different outputs.
- `tests/rpc_test.rs` — `tower::ServiceExt::oneshot` against the axum Router built by `RpcServer`. Scenarios:
  - `POST /pair` returns 200 with valid `PairResponse` JSON; session state acquires a token.
  - `GET /status` returns `paired: false` initially, `paired: true` after `/pair`.
- `tests/gateway_client_test.rs` — small in-process WS test gateway (`tokio::net::TcpListener` + manual handshake) that returns `ServerHello` or `Error`. Test `GatewayClient::connect`:
  - Happy path: ServerHello → returns Ok with `session_id` set.
  - Error path: gateway returns `ControlMessage::Error` → `connect` returns Err.
  - Wrong first frame: gateway returns Binary → returns Err.
- `tests/leakage_test.rs` — three scenarios:
  - `pair_response_does_not_leak_api_key`: spawn RpcServer, call `/pair`, assert response JSON does not contain `state.api_key.as_str()`. (The state holds api_key for gateway connect; the RPC response must not echo it.)
  - `tracing_does_not_leak_api_key_on_gateway_connect_failure`: with a tracing capture layer, attempt `GatewayClient::connect` to a closed port; assert no captured line contains api_key.
  - `debug_session_state_does_not_leak_token`: `format!("{:?}", session_state)` does not contain the token value.

### 3.4 Web (`web/`)

**Tooling setup:**

- Install `vitest`, `@vitest/ui` (optional), `@testing-library/react`, `@testing-library/jest-dom`, `@testing-library/user-event`, `jsdom`.
- `vite.config.ts` gets a `test` block with `environment: 'jsdom'`, `globals: true`, `setupFiles: ['./src/test/setup.ts']`.
- `src/test/setup.ts` imports `@testing-library/jest-dom`, installs a `WebSocket` stub on `globalThis`.
- `package.json` adds `"test": "vitest run"` and `"test:watch": "vitest"`.

**WebSocket mock strategy:**

A minimal `MockWebSocket` class in `src/test/mock-ws.ts` matching the parts of the WebSocket interface that `WsClient` uses: `addEventListener('open' | 'close' | 'error' | 'message')`, `send`, `close`, `readyState`. Tests instantiate it directly or substitute `globalThis.WebSocket = MockWebSocket`. We deliberately do not use `mock-socket` to avoid the dependency and keep the surface obvious.

**Files to add:**

- `src/lib/ws_client.test.ts` — open/close lifecycle, message dispatch by type, send queue while not-open, send after open.
- `src/lib/protocol.test.ts` — parse each `ControlMessage` variant; reject malformed JSON.
- `src/store/session.test.ts` — Zustand store: initial state, setters, derived selectors.
- `src/hooks/useWebSocket.test.ts` — hook subscribes/unsubscribes, exposes connection state.
- `src/hooks/useReconnect.test.ts` — exponential backoff math, max-attempt cap, cancel on unmount. Use `vi.useFakeTimers()`.
- `src/hooks/useVisualViewport.test.ts` — height changes propagate; cleanup on unmount.
- `src/components/ActionBar/keys.test.ts` — key map produces correct escape sequences (Esc, Tab, arrows, Ctrl+C, Enter).
- `src/components/ActionBar/ActionBar.test.tsx` — render renders all keys; click emits `onKey` callback.
- `src/components/ErrorBoundary/ErrorBoundary.test.tsx` — catches a thrown error, renders fallback, logs to `console.error` but only with sanitized info.
- `src/components/Layout/MobileLayout.test.tsx` — viewport-driven layout class swaps.
- `src/pages/NotFoundPage.test.tsx`, `src/pages/ErrorPage.test.tsx` — render content.
- `src/lib/leakage.test.ts` — secret-leakage assertions:
  - After hydrating a session token from URL, `localStorage.length === 0` and `sessionStorage.length === 0`.
  - `window.history.state` does not include the token after navigation.
  - `ws_client` does not log raw `e.data` (production code change required, see below). The test triggers a malformed frame, asserts `console.error` was called, asserts no captured argument contains a 43-char base64url-shaped substring.

**Production code change required for leakage test #3:**

`ws_client.ts:32` currently logs `e.data` verbatim. Replace with logging only the parse error and a marker like `"<raw frame omitted>"`. The leakage test asserts this behavior.

### 3.5 Cross-cutting leakage prevention (5th area)

Beyond the per-area leakage tests above, this is the policy these tests enforce:

1. **No secret in `Debug`/`Display`.** Manual `Debug` in `SessionToken`/`ApiKey`. Derived `Debug` on protocol structs uses field `Debug`, which (because of #1) is safe.
2. **No secret in tracing.** Every test with a tracing capture asserts the relevant secret string is absent from captured output. Gateway and wrapper both have such tests.
3. **No secret echoed in error responses.** Gateway error messages are static strings (`"invalid api_key"`, `"invalid token"`), never templated with user input.
4. **No secret in browser persistence.** Web tests check `localStorage`, `sessionStorage`, `window.history.state`.
5. **No raw frame data in console.** Web `ws_client` change above. Tests assert no 43-char base64url-shaped substring appears in captured console output during a normal session.

---

## 4. Refactor risk and mitigations

| Refactor | Risk | Mitigation |
| --- | --- | --- |
| Shared `define_secret_token!` macro | API change visible to gateway/wrapper/pair | Macro emits same public surface (`parse`, `generate`, `as_str`, `ct_eq`); rename `constant_time_eq` → `ct_eq` is the only breaking change. Update 3 call sites. |
| `serde(try_from = "String", …)` | Existing JSON in tests/fixtures may fail to deserialize if values are invalid | Existing tests all use `generate()` outputs — all 43-char base64url. No fixture file under `crates/` uses hardcoded short strings. |
| `Zeroize` on `String` | `String` does not implement `Zeroize` directly | Use `zeroize::Zeroizing<String>` as the inner type, or use `zeroize::Zeroize` derive on a `String` newtype with manual impl. Choose `Zeroizing<String>` — standard pattern. |
| Wrapper Bridge generics | Touches main.rs wiring | Adapter impls in `bridge.rs`. Main only changes type of `bridge::run` call (now `bridge::run(stream_adapter, sink_adapter, pty_adapter)`). |
| `subtle::ConstantTimeEq` derive | Crate must be in workspace deps | Add to workspace. No breaking change beyond rename. |
| Web `ws_client` no longer logs `e.data` | Reduced debug info during development | Replace with structured log of error code + frame length, not content. |

---

## 5. Implementation phases

Each phase is a single commit. Conventional commit prefixes per repo convention.

### Phase 1 — Gateway e2e (M2.6 closure)

- Add `crates/claude-phone-gateway/tests/e2e_test.rs` (verbatim from master plan).
- Add `crates/claude-phone-gateway/tests/leakage_test.rs` (3 scenarios in §3.1).
- Add dev-deps to workspace: `portpicker`, ensure `tracing-subscriber` test feature.
- Verify `cargo test -p claude-phone-gateway`.
- Commit: `test(gateway): e2e bridge + leakage assertions (M2.6, M9.4 partial)`

### Phase 2 — Shared types refactor + tests

- Add `zeroize`, `subtle` to workspace `Cargo.toml`.
- Rewrite `crates/claude-phone-shared/src/token.rs` using `define_secret_token!` macro covering M9.4 items #1–#12.
- Update 3 call sites (gateway, wrapper, pair) for `constant_time_eq` → `ct_eq` rename and any `TokenError` variant matches.
- Update existing `tests/token_test.rs` with edge cases and variant pinning.
- Update existing `tests/protocol_test.rs` with roundtrip per variant.
- Add new `tests/leakage_test.rs`.
- Remove `serde_json` from `[dependencies]` (keep in `[dev-dependencies]`).
- Verify `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.
- Commit: `feat(shared)!: secret-safe token types — manual Debug, zeroize, subtle, validated serde (M9.4 #1-12)`

### Phase 3 — Wrapper refactor + tests for 9 modules

- Add `BridgeStream`/`BridgeSink`/`BridgePty` traits and adapter impls in `bridge.rs`.
- Refactor `bridge::run_via_locked` → `bridge::run<S, K, P>` (generics).
- Update `main.rs` to construct adapters.
- Verify `docs/development.md` smoke test path still works (cargo run, plugin /phone, dummy claude session). If headless, simulate by running `cargo run -- --help` and `cargo build --bin claude-phone`.
- Add the 9 test files in §3.3 (cli, config, session, bridge, tty, pty, qr, rpc, gateway_client) + `leakage_test.rs`.
- Verify `cargo test -p claude-phone-wrapper` and `cargo clippy --workspace -- -D warnings`.
- Commit: `test(wrapper): cover 9 modules + leakage; bridge gains trait adapters`

### Phase 4 — Web Vitest setup + tests

- Install Vitest + Testing Library deps via pnpm.
- Add `vite.config.ts` test block, `src/test/setup.ts`, `src/test/mock-ws.ts`.
- Modify `web/src/lib/ws_client.ts` to not log raw `e.data` (leakage fix; see §3.4).
- Add all test files in §3.4 (13 files including `leakage.test.ts`).
- Update `package.json` scripts (`test`, `test:watch`).
- Verify `pnpm test` green.
- Commit: `test(web): Vitest + RTL coverage; ws_client no longer logs raw frame data`

### Phase 5 — CI integration

- Inspect `.github/workflows/` for existing pipelines (commit `e7861d9` added Rust + web pipelines).
- Add `cargo test --workspace` step if missing.
- Add `cd web && pnpm install && pnpm test` step if missing.
- Verify the workflow file syntax with `gh workflow view` or local YAML lint.
- Commit: `ci: run new test suites for Rust workspace and web` (only if changes needed)

### Final — Push and verify

- `git status` clean, all phases committed.
- `git push origin main`.
- Verify CI green on GitHub (poll `gh run list --limit 1` after push).
- Update `MEMORY.md` entries: mark M9.4 shared-types deferral resolved (or remove that memory file).

---

## 6. CI integration details

The existing CI from commit `e7861d9` will be inspected during Phase 5. Expected steps to add or verify:

```yaml
# .github/workflows/ci.yml (Rust job)
- name: Test
  run: cargo test --workspace --all-features

# Web job
- name: Test
  working-directory: web
  run: pnpm test
```

Both already may exist; Phase 5 is a no-op commit-wise if so. The `test` job becomes a required check for the main branch via repo settings — out of scope for this push (the user can flip the toggle manually).

---

## 7. Open questions

None blocking. The user explicitly delegated all remaining tactical decisions to autonomous execution ("zrob bezpiecznie i z features").

If during implementation a decision arises that materially changes scope (e.g., a refactor turns out to be larger than estimated, or a test framework limitation forces a different approach), the autonomous executor records it as a deviation note in the commit message body for the user to review on return.

---

## 8. References

- Master plan: `C:\Users\mrzyg\.claude\plans\expressive-snuggling-parasol.md` (Task 2.6, line 2712)
- Memory: `project_security_deferrals.md` — full list of M9.4 items #1–#12
- Memory: `project_claude_phone.md` — context, stack, enterprise-grade preference
- ADR: `docs/adr/0001-rust-for-network.md`, `0002-pty-not-sdk.md`, `0003-xterm-for-rendering.md`
- Security: `docs/security.md` — existing threat model
