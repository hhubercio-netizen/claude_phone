use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::sync::{Mutex, Notify};

use claude_phone_shared::SessionToken;

/// Direction-tagged frame flowing through the bridge.
#[derive(Debug, Clone)]
pub enum Frame {
    /// Binary PTY bytes (wrapper → phone) or keystrokes (phone → wrapper)
    Binary(Vec<u8>),
    /// Text JSON control message
    Text(String),
}

/// Cap on the ring-buffer of binary frames held while no phone is attached.
/// Sized for terminal scrollback (~hundreds of lines of typical xterm output).
/// Oldest entries are evicted when the cap is hit — terminals always show the
/// most recent output anyway.
pub const PHONE_BUFFER_BYTES_CAP: usize = 64 * 1024;
pub const PHONE_BUFFER_FRAMES_CAP: usize = 256;

/// State of the wrapper → phone direction of one session. Holds either an
/// active phone sender, or a ring buffer of pending binary frames when no
/// phone is attached. Text (control) frames are *not* buffered — they are
/// transient signals that would be confusing to replay.
#[derive(Debug, Default)]
pub struct PhoneChannel {
    sender: Option<mpsc::Sender<Frame>>,
    buffer: VecDeque<Vec<u8>>,
    buffer_bytes: usize,
}

impl PhoneChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_attached(&self) -> bool {
        self.sender.is_some()
    }

    /// Clone of the active sender, or None when no phone is attached.
    pub fn sender(&self) -> Option<mpsc::Sender<Frame>> {
        self.sender.clone()
    }

    /// Detach the current sender. The phone WS task is shutting down so its
    /// rx will be dropped; we clear our half too.
    pub fn detach(&mut self) {
        self.sender = None;
    }

    /// Append a binary frame to the buffer, evicting oldest entries to keep
    /// total bytes <= PHONE_BUFFER_BYTES_CAP and frame count <=
    /// PHONE_BUFFER_FRAMES_CAP.
    pub fn push_buffered(&mut self, bytes: Vec<u8>) {
        self.buffer_bytes += bytes.len();
        self.buffer.push_back(bytes);
        while self.buffer_bytes > PHONE_BUFFER_BYTES_CAP
            || self.buffer.len() > PHONE_BUFFER_FRAMES_CAP
        {
            if let Some(old) = self.buffer.pop_front() {
                self.buffer_bytes -= old.len();
            } else {
                break;
            }
        }
    }

    /// Attach a new sender and atomically replay all buffered binary frames
    /// into it. Returns the count of frames replayed (for tracing/tests).
    ///
    /// Replay happens *before* the sender is stored so any subsequent wrapper
    /// write (which clones the sender from this slot) is guaranteed to arrive
    /// in the channel after the replayed history.
    pub fn attach_and_replay(&mut self, new_sender: mpsc::Sender<Frame>) -> usize {
        let drained: Vec<Vec<u8>> = self.buffer.drain(..).collect();
        self.buffer_bytes = 0;
        let mut replayed = 0;
        for bytes in drained {
            // try_send because we hold the slot lock and we just created
            // the channel with 256 cap. If it ever returns Err, drop the
            // entry — buffer history is best-effort, not authoritative.
            if new_sender.try_send(Frame::Binary(bytes)).is_err() {
                break;
            }
            replayed += 1;
        }
        self.sender = Some(new_sender);
        replayed
    }

    #[cfg(test)]
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    #[cfg(test)]
    pub fn buffer_bytes(&self) -> usize {
        self.buffer_bytes
    }
}

/// In-memory state for one live session.
/// Wrapper and phone each get a Sender; the bridge task forwards between them.
pub struct Session {
    pub id: String,
    pub token: SessionToken,
    pub to_wrapper: mpsc::Sender<Frame>,
    pub to_phone: Arc<Mutex<PhoneChannel>>,
    /// Wall-clock of the most recent moment a phone was attached to this
    /// session. Updated on attach AND on detach so an actively-connected
    /// phone keeps the timestamp fresh; the sweeper uses this plus the
    /// `is_attached()` flag to decide expiry.
    pub last_phone_seen: Arc<Mutex<Instant>>,
    /// Server-initiated cancellation. The idle sweeper sets `cancelled` and
    /// wakes any task awaiting `cancel.notified()`. The flag handles the
    /// race where the sweeper fires before a task has actually started
    /// polling — late starters check the flag and bail immediately.
    pub cancel: Arc<Cancel>,
}

/// Notify-with-flag, structured so a task that registers AFTER the sweeper
/// fires still observes cancellation. `Notify::notify_waiters()` alone is
/// edge-triggered: anyone not yet polling at the moment of the notify
/// would miss it. Pairing it with an `AtomicBool` makes the signal sticky.
#[derive(Debug, Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Set the flag and wake every current waiter. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Future that resolves as soon as cancel is observed. Safe to await
    /// even if cancel already fired — the flag short-circuits the wait.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl Session {
    pub fn new(token: SessionToken, to_wrapper: mpsc::Sender<Frame>) -> Self {
        Self {
            id: short_id(),
            token,
            to_wrapper,
            to_phone: Arc::new(Mutex::new(PhoneChannel::new())),
            last_phone_seen: Arc::new(Mutex::new(Instant::now())),
            cancel: Arc::new(Cancel::new()),
        }
    }

    /// Update `last_phone_seen` to `now`. Call at phone attach and detach.
    pub async fn touch_phone(&self) {
        *self.last_phone_seen.lock().await = Instant::now();
    }
}

fn short_id() -> String {
    use rand::Rng;
    let n: u64 = rand::thread_rng().gen();
    format!("{n:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_buffered_caps_total_bytes() {
        let mut ch = PhoneChannel::new();
        for _ in 0..100 {
            ch.push_buffered(vec![0u8; 1024]); // 100 KB total pushed
        }
        assert!(ch.buffer_bytes() <= PHONE_BUFFER_BYTES_CAP);
    }

    #[test]
    fn push_buffered_evicts_oldest() {
        let mut ch = PhoneChannel::new();
        ch.push_buffered(vec![b'A'; 50_000]);
        ch.push_buffered(vec![b'B'; 50_000]);
        // 100KB pushed, cap is 64KB. The first 'A' chunk must have been
        // evicted; only the second remains (50KB <= 64KB).
        assert_eq!(ch.buffer_bytes(), 50_000);
        assert_eq!(ch.buffer_len(), 1);
    }

    #[test]
    fn push_buffered_caps_frame_count() {
        let mut ch = PhoneChannel::new();
        for _ in 0..(PHONE_BUFFER_FRAMES_CAP + 50) {
            ch.push_buffered(vec![1u8]);
        }
        assert!(ch.buffer_len() <= PHONE_BUFFER_FRAMES_CAP);
    }

    #[tokio::test]
    async fn attach_and_replay_drains_buffer_into_new_sender() {
        let mut ch = PhoneChannel::new();
        ch.push_buffered(b"hello ".to_vec());
        ch.push_buffered(b"world".to_vec());

        let (tx, mut rx) = mpsc::channel::<Frame>(16);
        let n = ch.attach_and_replay(tx);
        assert_eq!(n, 2);
        assert_eq!(ch.buffer_len(), 0);
        assert_eq!(ch.buffer_bytes(), 0);

        let f1 = rx.recv().await.unwrap();
        let f2 = rx.recv().await.unwrap();
        match (f1, f2) {
            (Frame::Binary(a), Frame::Binary(b)) => {
                assert_eq!(a, b"hello ");
                assert_eq!(b, b"world");
            }
            _ => panic!("expected two binary frames"),
        }
    }

    #[tokio::test]
    async fn detach_clears_sender_only() {
        let mut ch = PhoneChannel::new();
        let (tx, _rx) = mpsc::channel::<Frame>(16);
        ch.attach_and_replay(tx);
        ch.push_buffered(b"x".to_vec()); // won't actually buffer (attached)
                                         // — but this test only checks detach behavior.
        ch.detach();
        assert!(!ch.is_attached());
        assert!(ch.sender().is_none());
    }
}
