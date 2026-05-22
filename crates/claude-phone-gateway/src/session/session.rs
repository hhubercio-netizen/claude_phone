use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::Mutex;

use claude_phone_shared::SessionToken;

/// Direction-tagged frame flowing through the bridge.
#[derive(Debug, Clone)]
pub enum Frame {
    /// Binary PTY bytes (wrapper → phone) or keystrokes (phone → wrapper)
    Binary(Vec<u8>),
    /// Text JSON control message
    Text(String),
}

/// In-memory state for one live session.
/// Wrapper and phone each get a Sender; the bridge task forwards between them.
pub struct Session {
    pub id: String,
    pub token: SessionToken,
    pub to_wrapper: mpsc::Sender<Frame>,
    pub to_phone: Arc<Mutex<Option<mpsc::Sender<Frame>>>>,
}

impl Session {
    pub fn new(token: SessionToken, to_wrapper: mpsc::Sender<Frame>) -> Self {
        Self {
            id: short_id(),
            token,
            to_wrapper,
            to_phone: Arc::new(Mutex::new(None)),
        }
    }
}

fn short_id() -> String {
    use rand::Rng;
    let n: u64 = rand::thread_rng().gen();
    format!("{n:016x}")
}
