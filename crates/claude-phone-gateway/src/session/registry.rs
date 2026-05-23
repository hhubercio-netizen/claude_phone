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
        }
    }

    pub async fn register_wrapper(&self, token: SessionToken) -> RegisterResult {
        let key = token.as_str().to_string();

        // Soft pre-check against max_sessions. The atomic check happens inside
        // the Entry below (where len() is read again with the shard lock held)
        // to keep the bound under concurrent registrations.
        if self.inner.len() >= self.max_sessions {
            return Err(GatewayError::Internal(anyhow::anyhow!(
                "max sessions reached"
            )));
        }

        let (tx_to_wrapper, rx_from_phone) = mpsc::channel::<Frame>(256);
        let session = Arc::new(Session::new(token, tx_to_wrapper));

        match self.inner.entry(key) {
            Entry::Occupied(_) => Err(GatewayError::SessionTaken),
            Entry::Vacant(v) => {
                if v.key().len() > 64 {
                    return Err(GatewayError::InvalidToken);
                }
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
                // A phone is already attached. Refuse the new one rather than
                // silently stealing the session — prevents takeover by anyone
                // who knows the token while the original holder is connected.
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
    pub async fn drop_session(&self, token: &SessionToken) {
        if let Some((_, session)) = self.inner.remove(token.as_str()) {
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
        self.inner.remove(token.as_str());
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
