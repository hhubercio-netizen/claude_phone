use std::sync::Arc;

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
        if self.inner.contains_key(&key) {
            return Err(GatewayError::SessionTaken);
        }
        if self.inner.len() >= self.max_sessions {
            return Err(GatewayError::Internal(anyhow::anyhow!(
                "max sessions reached"
            )));
        }

        let (tx_to_wrapper, rx_from_phone) = mpsc::channel::<Frame>(256);
        let session = Arc::new(Session::new(token, tx_to_wrapper));
        self.inner.insert(key, session.clone());

        Ok(WrapperHandle {
            session,
            rx: rx_from_phone,
        })
    }

    pub async fn attach_phone(&self, token: &SessionToken) -> Result<PhoneHandle, GatewayError> {
        let key = token.as_str().to_string();
        let session = self
            .inner
            .get(&key)
            .ok_or(GatewayError::SessionNotFound)?
            .clone();

        let (tx_to_phone, rx_from_wrapper) = mpsc::channel::<Frame>(256);
        {
            let mut slot = session.to_phone.lock().await;
            *slot = Some(tx_to_phone);
        }

        Ok(PhoneHandle {
            session,
            rx: rx_from_wrapper,
        })
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
