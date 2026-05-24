use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::mpsc;

use claude_phone_shared::SessionToken;

use super::session::{Frame, Session};
use crate::error::GatewayError;

pub struct SessionRegistry {
    inner: Arc<DashMap<String, Arc<Session>>>,
    max_sessions: usize,
    // TM-CODE.4: atomic reservation counter prevents shard-race over-allocation
    // against `max_sessions`. DashMap holds per-shard locks; without a global
    // counter, two concurrent register_wrapper calls whose tokens hash to
    // different shards could both pass a `len()` check and both insert.
    active_count: AtomicUsize,
}

pub struct WrapperHandle {
    pub session: Arc<Session>,
    pub rx: mpsc::Receiver<Frame>,
}

pub struct PhoneHandle {
    pub session: Arc<Session>,
    pub rx: mpsc::Receiver<Frame>,
}

pub type RegisterResult = Result<WrapperHandle, GatewayError>;

impl SessionRegistry {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            max_sessions,
            active_count: AtomicUsize::new(0),
        }
    }

    pub async fn register_wrapper(&self, token: SessionToken) -> RegisterResult {
        // TM-CODE.4: atomic reservation BEFORE allocating per-session state.
        // Reserve the slot first; only proceed if we are within the cap.
        let prev = self.active_count.fetch_add(1, Ordering::SeqCst);
        if prev >= self.max_sessions {
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            return Err(GatewayError::Internal(anyhow::anyhow!(
                "max sessions reached"
            )));
        }

        let key = token.as_str().to_string();
        if key.len() > 64 {
            // TM-CODE.4: give the reservation back since we are rejecting.
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            return Err(GatewayError::InvalidToken);
        }

        let (tx_to_wrapper, rx_from_phone) = mpsc::channel::<Frame>(256);
        let session = Arc::new(Session::new(token, tx_to_wrapper));

        match self.inner.entry(key) {
            Entry::Occupied(_) => {
                // TM-CODE.4: slot is taken — return reservation.
                self.active_count.fetch_sub(1, Ordering::SeqCst);
                Err(GatewayError::SessionTaken)
            }
            Entry::Vacant(v) => {
                v.insert(session.clone());
                Ok(WrapperHandle {
                    session,
                    rx: rx_from_phone,
                })
            }
        }
    }

    pub async fn attach_phone(&self, token: &SessionToken) -> Result<PhoneHandle, GatewayError> {
        let key = token.as_str().to_string();
        let session = self
            .inner
            .get(&key)
            .ok_or(GatewayError::SessionNotFound)?
            .clone();

        let (tx_to_phone, rx_from_wrapper) = mpsc::channel::<Frame>(256);
        let replayed = {
            let mut slot = session.to_phone.lock().await;
            if slot.is_attached() {
                // TM-AUTH.3: single-phone-per-session is the D1 default — a
                // second concurrent phone attempt is REFUSED, NOT kicked
                // through. The asymmetric choice matters: kick-previous would
                // let anyone who learned the token (shoulder-surf, screen-
                // share leak, browser history sync) silently boot the
                // legitimate user and impersonate them. Refuse-second means
                // the worst an attacker with a stolen token can do is
                // generate noise — they cannot displace the live operator.
                // The reattach-after-detach path is intentionally separate
                // (`detach_then_reattach_after_disconnect` test) so a real
                // user reconnect after network loss still works.
                return Err(GatewayError::SessionTaken);
            }
            slot.attach_and_replay(tx_to_phone)
        };
        // Reset the idle clock — a fresh phone attach is the "see" event the
        // sweeper uses to decide expiry.
        session.touch_phone().await;
        if replayed > 0 {
            tracing::debug!(replayed, "replayed buffered frames to phone");
        }

        Ok(PhoneHandle {
            session,
            rx: rx_from_wrapper,
        })
    }

    /// Snapshot of all sessions and their idle metadata. Used by the
    /// background sweeper; returns cloned `Arc`s so the DashMap is not held
    /// locked while the caller does async work.
    pub fn sessions_snapshot(&self) -> Vec<Arc<Session>> {
        self.inner.iter().map(|v| v.clone()).collect()
    }

    /// Drop a session by token and fire its `cancel` so any wrapper/phone WS
    /// tasks bound to it tear down. Idempotent.
    // TM-AUTH.5: forgetting a token is unconditional — remove the entry then
    // cancel any task still bound to it. The sweeper (TM-AUTH.11) and the
    // wrapper-exit path both funnel through here.
    pub async fn drop_session(&self, token: &SessionToken) {
        if let Some((_, session)) = self.inner.remove(token.as_str()) {
            // TM-CODE.4: keep active_count in sync with inner. Only decrement
            // on actual removal so double-calls remain idempotent.
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            session.cancel.cancel();
            tracing::info!(session_id = %session.id, "session dropped by sweeper");
        }
    }

    /// Sweep: drop every session that has had no attached phone for at
    /// least `idle_timeout`. Sessions with an attached phone are never
    /// considered expired no matter how long the phone has been quiet.
    /// Returns the number of sessions dropped (for tracing/tests).
    pub async fn sweep_expired(&self, idle_timeout: Duration) -> usize {
        let now = Instant::now();
        let mut dropped = 0;
        for session in self.sessions_snapshot() {
            let still_attached = session.to_phone.lock().await.is_attached();
            if still_attached {
                continue;
            }
            let last_seen = *session.last_phone_seen.lock().await;
            if now.duration_since(last_seen) >= idle_timeout {
                self.drop_session(&session.token).await;
                dropped += 1;
            }
        }
        dropped
    }

    pub fn lookup(&self, token: &SessionToken) -> Option<Arc<Session>> {
        self.inner.get(token.as_str()).map(|v| v.clone())
    }

    pub fn remove(&self, token: &SessionToken) {
        if self.inner.remove(token.as_str()).is_some() {
            // TM-CODE.4: decrement on actual removal only.
            self.active_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
