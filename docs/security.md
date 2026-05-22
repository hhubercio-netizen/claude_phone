# Security threat model

## Trust boundaries

1. **Dev machine ↔ home gateway**: encrypted via TLS (Cloudflare or origin
   cert). Auth: per-user API key.
2. **Phone ↔ home gateway**: encrypted via TLS. Auth: 256-bit session token in
   URL (capability URL).
3. **Wrapper ↔ Claude (child process)**: PTY, same trust domain as the user.
4. **Plugin ↔ wrapper RPC**: 127.0.0.1 only, no auth (treated as same-host).

## Threats and mitigations

| #  | Threat                                                                | Mitigation                                                                                                                                                                       |
|----|-----------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| T1 | Attacker guesses the session URL                                       | 256-bit token (43 base64url chars). Brute force impossible at any practical rate.                                                                                                |
| T2 | Attacker grabs API key from `~/.config/claude-phone/config.toml`      | File mode 0600 (user-only); user is responsible for protecting their machine. Compromise of the dev machine = full game over anyway.                                             |
| T3 | Attacker on the same WiFi MITMs the wrapper↔gateway connection         | All traffic is WSS (TLS). Cloudflare cert is trusted.                                                                                                                            |
| T4 | Attacker discovers the local RPC port and spams `/pair`                | The port is random and on 127.0.0.1 only. No remote access possible unless attacker is already root on the dev machine.                                                          |
| T5 | Stolen phone has an open Claude Phone tab                              | Mitigation v2: server idle timeout (e.g., 15 min) closes phone WS; user must scan a fresh QR. Currently no mitigation — accepted risk.                                            |
| T6 | Replay of a captured WS frame                                          | Token is single-session; after wrapper exits, server forgets it. Frames within a session are not authenticated individually (TLS prevents replay across sessions).                |
| T7 | XSS in the React app                                                  | React's default escaping + we never `dangerouslySetInnerHTML` PTY contents. xterm.js operates on a canvas/DOM that does its own escaping.                                         |
| T8 | DoS by opening many WS connections                                    | Gateway enforces `max_sessions` (default 32). Cloudflare rate limiting recommended (see `docs/deployment.md`).                                                                   |
| T9 | Wrapper sends garbage to the phone                                    | The phone is treated as semi-trusted: if it sends malformed control messages, the gateway logs and drops. PTY data is opaque.                                                     |
| T10 | Cloudflare or DNS compromise                                          | Out of scope. User accepts that trusting Cloudflare ↔ origin is part of the model.                                                                                              |

## Deferred hardening (from M1.1 code review)

The `SessionToken` / `ApiKey` types in `claude-phone-shared` were implemented
to spec but the code review surfaced standard-practice secret-handling gaps.
Tracked as follow-up work, to be addressed in a dedicated security pass:

1. **`#[serde(transparent)]` bypasses `parse()` validation.** Any JSON string
   deserializes into a "valid" token at the wire boundary, defeating the
   newtype's invariants. **Fix:** use `#[serde(try_from = "String", into = "String")]`
   with `TryFrom<String>` calling `parse()`.
2. **Derived `Debug` prints the secret.** `tracing::info!(?token)` would leak
   the token into logs. **Fix:** custom `Debug` impl returning `"SessionToken(***)"`.
3. **No `zeroize` on drop.** Freed heap memory still contains the secret —
   recoverable from core dumps / swap. **Fix:** add `zeroize` crate, derive
   `ZeroizeOnDrop`.
4. **Hand-rolled `constant_time_eq` lacks a compiler barrier.** LLVM is not
   contractually required to keep it branch-free. **Fix:** use `subtle::ConstantTimeEq`.
5. **`TokenError` has two variants (`InvalidLength`, `InvalidChars`).** Spec
   asked for opaque errors. **Fix:** collapse to a single `Invalid` variant.
6. **`parse()` uses `.chars().all(...)`** which short-circuits — timing oracle
   for the position of the first invalid byte. **Fix:** non-short-circuiting fold.
7. **Tests assert `is_err()` only**, not the specific variant. **Fix:**
   `matches!(r, Err(TokenError::Invalid))` etc.
8. **Missing edge cases**: length 42, length 44, empty string, padded base64,
   `ApiKey::parse` is entirely untested. **Fix:** add tests.
9. **`constant_time_eq` test only covers the equal case**; the inequality
   contract (the actual security purpose) is untested. **Fix:** test
   differing-token comparisons.
10. **`ApiKey` is a verbatim copy of `SessionToken`** — any future fix must be
    applied to both. **Fix:** `macro_rules! define_secret_token!` to share the
    impl mechanically.
11. **`serde_json` is in both `[dependencies]` and `[dev-dependencies]`** of
    `claude-phone-shared`. **Fix:** keep only `dev-dependencies` (the library
    does not use `serde_json` at runtime).
12. **`LEN = 43` is hardcoded** rather than derived from `BYTES = 32`. **Fix:**
    `const BYTES: usize = 32; const LEN: usize = (BYTES * 4 + 2) / 3;`.

## Future work

- Phone-side passcode prompt (4-digit, in-browser).
- Per-session idle timeout enforced by gateway.
- Audit log of phone connect/disconnect events.
- mTLS for wrapper↔gateway.
- Replay buffer on gateway so reconnecting phones see recent PTY history.
