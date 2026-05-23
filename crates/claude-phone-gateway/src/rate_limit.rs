//! Per-IP rate limiters wired into the HTTP layer and (later) the WS auth path.
//!
//! This commit lands only the TM-RATE.1 constants (per-IP HTTP cap). The
//! per-IP auth-failure tracker (TM-RATE.2) and per-connection sliding-window
//! limiter (TM-RATE.3) land in subsequent commits — splitting them keeps
//! each diff small and individually auditable.

// TM-RATE.1 — per-IP HTTP-layer cap.
//
// 5 requests per second per IP, burst of 10. Sliding window via governor's
// leaky-bucket cell rate limiter. A WebSocket upgrade is one HTTP request:
// sustained 5/s lets a legitimate operator reconnect on a flaky mobile
// network without tripping the limiter, while a flood-style attacker hits
// the wall at the first burst.
//
// Both values are exposed as `pub const` so tests can refer to them by
// symbol rather than literal — if the policy ever changes the rate-limit
// integration test (`tests/rate_limit.rs::per_ip_governor_returns_429_under_burst`)
// stays in sync automatically.
pub const PER_IP_REQ_PER_SEC: u64 = 5;
pub const PER_IP_BURST: u32 = 10;
