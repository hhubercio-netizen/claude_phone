//! Per-IP rate limiters wired into the HTTP layer and the WS auth path.
//!
//! This file owns three of the rate-limit catalog entries:
//! - TM-RATE.1: per-IP HTTP-layer cap (constants only; the limiter itself
//!   is materialised by `tower_governor::GovernorLayer` in `http.rs`).
//! - TM-RATE.2: per-IP auth-failure tracker with exponential backoff.
//! - TM-RATE.3: per-connection sliding-window message limiter.
//!
//! Keeping them in one module is deliberate — they share IP-keyed lookup
//! and a single auditor can review the policy from one file rather than
//! tracing per-file scatter.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

// --- TM-RATE.1 -------------------------------------------------------------

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

// --- TM-RATE.2 -------------------------------------------------------------

// TM-RATE.2 — per-IP auth-failure tracker with exponential backoff.
//
// HTTP rate limit (TM-RATE.1) bounds raw request rate to ~5 r/s burst 10 —
// enough to brute-force ~432 000 keys/day per IP unhindered. 256-bit
// API keys make that uninteresting mathematically, but the tracker is
// still useful because (a) it short-circuits the WS upgrade for known-bad
// IPs before any further auth work happens, and (b) it surfaces a strong
// signal for operator alerting.
pub const AUTH_FAIL_THRESHOLD: u32 = 10;
pub const AUTH_FAIL_WINDOW: Duration = Duration::from_secs(60);
pub const AUTH_BACKOFF_BASE_SECS: u64 = 2;
pub const AUTH_BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Debug, Default)]
struct AuthState {
    failures: VecDeque<Instant>,
    escalations: u32,
    locked_until: Option<Instant>,
}

/// Process-wide auth-failure tracker. Cheap to clone — the inner state is
/// `Arc<DashMap>` so every handler thread sees the same view.
#[derive(Clone, Default)]
pub struct AuthRateLimiter {
    inner: Arc<DashMap<IpAddr, AuthState>>,
}

impl AuthRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if this IP is currently inside its lockout window.
    /// Read-only; does not mutate state.
    pub fn is_locked(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        if let Some(state) = self.inner.get(&ip) {
            if let Some(until) = state.locked_until {
                return now < until;
            }
        }
        false
    }

    /// Record a failed WrapperHello auth. If the rolling-window count
    /// reaches `AUTH_FAIL_THRESHOLD`, set a lockout for an exponentially
    /// growing window: `2^escalations` seconds, capped at one hour. The
    /// escalation counter only resets on a successful auth, so a long
    /// sequence of attempts ratchets up cumulatively.
    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut entry = self.inner.entry(ip).or_default();
        let cutoff = now.checked_sub(AUTH_FAIL_WINDOW).unwrap_or(now);
        while entry.failures.front().is_some_and(|t| *t < cutoff) {
            entry.failures.pop_front();
        }
        entry.failures.push_back(now);
        if entry.failures.len() as u32 >= AUTH_FAIL_THRESHOLD {
            entry.escalations = entry.escalations.saturating_add(1);
            let secs = AUTH_BACKOFF_BASE_SECS
                .saturating_pow(entry.escalations)
                .min(AUTH_BACKOFF_CAP_SECS);
            entry.locked_until = Some(now + Duration::from_secs(secs));
            entry.failures.clear();
        }
    }

    /// Record a successful auth. Clears the rolling window and resets the
    /// escalation counter so a previously-locked-but-now-legitimate
    /// operator is not re-locked by stale state.
    pub fn record_success(&self, ip: IpAddr) {
        if let Some(mut entry) = self.inner.get_mut(&ip) {
            entry.failures.clear();
            entry.escalations = 0;
            entry.locked_until = None;
        }
    }
}

// --- TM-RATE.3 -------------------------------------------------------------

// TM-RATE.3 — per-connection sliding-window message limiter.
//
// Holds the last `cap` arrival timestamps; the next message is rejected
// (and the connection torn down by the caller) if the oldest timestamp
// is less than 1 s old. Limit values:
// - phone → gateway: 100 msg/s (keystrokes — generous).
// - gateway → phone: 1000 msg/s (PTY bursts during heavy `claude` output).
pub const PHONE_TO_GW_MSG_PER_SEC: usize = 100;
pub const GW_TO_PHONE_MSG_PER_SEC: usize = 1000;

// --- TM-RATE.6 -------------------------------------------------------------

// TM-RATE.6 — slow-write defense.
//
// A malicious peer can accept TCP data infinitely slowly, leaving the
// outbound `sink.send()` future awaiting the writable signal forever.
// The bounded session channels (256 frames) protect the producer side,
// but a single sink that never drains still ties up that connection's
// task and one slot in the registry. A 5 s wall-clock timeout on every
// `sink.send()` lets us declare the connection dead and reclaim it.
//
// 5 s is loose enough that a genuinely backed-up but live peer on a
// flaky mobile uplink (one of our supported scenarios) is not killed
// during a transient stall, but strict enough that a hostile zero-rate
// reader is shut down well before its connection becomes a DoS lever.
pub const SINK_SEND_TIMEOUT: Duration = Duration::from_secs(5);

// --- TM-RATE.7 -------------------------------------------------------------

// TM-RATE.7 — post-hello idle / no-pong watchdog.
//
// The keepalive ping (30 s interval) only detects breakage in the WRITE
// direction. A peer that has half-closed its read side, or whose NAT
// state has been silently dropped, will keep absorbing our pings without
// ever responding. Without a pong-deadline we hold that connection's
// task and FDs indefinitely.
//
// On every received Pong we stamp `last_pong`. On every keepalive tick
// we check elapsed-since-last-pong; if it exceeds `PONG_DEADLINE` the
// connection is declared dead and the session is cancelled.
//
// 90 s is three keepalive intervals: a single dropped ping or pong is
// tolerated (mobile networks lose packets) but a sustained silence past
// three rounds is conclusive evidence the peer is gone.
pub const PONG_DEADLINE: Duration = Duration::from_secs(90);

#[derive(Debug)]
pub struct ConnRateLimiter {
    window: VecDeque<Instant>,
    cap: usize,
}

impl ConnRateLimiter {
    pub fn new(cap: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Returns `true` if the new arrival is within rate. The caller has
    /// already received the frame; this just tells them whether to keep
    /// the connection alive or close it as flooding.
    pub fn check(&mut self, now: Instant) -> bool {
        if self.window.len() == self.cap {
            if let Some(&oldest) = self.window.front() {
                if now.duration_since(oldest) < Duration::from_secs(1) {
                    return false;
                }
            }
            self.window.pop_front();
        }
        self.window.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn auth_limiter_unlocks_initially() {
        let l = AuthRateLimiter::new();
        assert!(!l.is_locked(ip(127, 0, 0, 1)));
    }

    #[test]
    fn auth_limiter_locks_after_threshold() {
        let l = AuthRateLimiter::new();
        let peer = ip(10, 0, 0, 5);
        for _ in 0..AUTH_FAIL_THRESHOLD {
            l.record_failure(peer);
        }
        assert!(
            l.is_locked(peer),
            "{AUTH_FAIL_THRESHOLD} failures must trigger lockout"
        );
    }

    #[test]
    fn auth_limiter_success_clears_lockout() {
        let l = AuthRateLimiter::new();
        let peer = ip(10, 0, 0, 6);
        for _ in 0..AUTH_FAIL_THRESHOLD {
            l.record_failure(peer);
        }
        assert!(l.is_locked(peer));
        l.record_success(peer);
        assert!(!l.is_locked(peer), "successful auth must clear lockout");
    }

    #[test]
    fn auth_limiter_isolates_distinct_ips() {
        let l = AuthRateLimiter::new();
        let attacker = ip(192, 168, 0, 1);
        let bystander = ip(192, 168, 0, 2);
        for _ in 0..AUTH_FAIL_THRESHOLD {
            l.record_failure(attacker);
        }
        assert!(l.is_locked(attacker));
        assert!(!l.is_locked(bystander), "lockout must not leak across IPs");
    }

    #[test]
    fn conn_rate_allows_up_to_cap_inside_one_second() {
        let mut r = ConnRateLimiter::new(5);
        let t0 = Instant::now();
        for i in 0..5 {
            assert!(r.check(t0 + Duration::from_millis(i * 10)));
        }
    }

    #[test]
    fn conn_rate_rejects_when_window_full_under_one_second() {
        let mut r = ConnRateLimiter::new(3);
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(r.check(t0 + Duration::from_millis(i * 10)));
        }
        assert!(
            !r.check(t0 + Duration::from_millis(100)),
            "4th arrival within 1s of oldest must be rejected"
        );
    }

    #[test]
    fn conn_rate_allows_after_window_slides() {
        let mut r = ConnRateLimiter::new(2);
        let t0 = Instant::now();
        assert!(r.check(t0));
        assert!(r.check(t0 + Duration::from_millis(10)));
        // The 3rd arrival, more than 1s after the oldest, must be allowed.
        assert!(r.check(t0 + Duration::from_millis(1500)));
    }
}
