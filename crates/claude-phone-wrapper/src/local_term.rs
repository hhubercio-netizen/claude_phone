use std::io::{Read, Write};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::pty::PtySession;

/// RAII guard that puts the controlling terminal into raw mode for the
/// lifetime of the value and restores cooked mode on drop. Required so the
/// wrapped `claude` TUI receives keystrokes byte-by-byte (no line buffering,
/// no local echo) just as it would if invoked directly.
///
/// On Windows, `crossterm` additionally enables `VIRTUAL_TERMINAL_PROCESSING`
/// on stdout and `VIRTUAL_TERMINAL_INPUT` on stdin so ANSI escape sequences
/// pass through cleanly in either direction.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|e| anyhow::anyhow!("enable_raw_mode failed: {e}"))?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Drives the host stdin → PTY and PTY → host stdout pumps.
///
/// `first_rx` is the broadcast receiver handed back by `PtySession::spawn`.
/// Using *that* particular receiver (rather than calling `subscribe()` here)
/// guarantees no early child output is lost between spawn time and the
/// moment this task starts polling.
///
/// Returns when either pump observes EOF, or the PTY exits. The caller is
/// expected to hold a [`RawModeGuard`] across the lifetime of this call.
pub async fn run(pty: Arc<PtySession>, first_rx: broadcast::Receiver<Vec<u8>>) {
    // PTY → stdout. We run inside spawn_blocking so writes can hit a busy
    // console without parking the tokio worker pool, and we drive the
    // broadcast receiver via Handle::block_on inside the same thread.
    let mut rx = first_rx;
    let pty_exit = pty.clone();
    let stdout_task = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        let mut stdout = std::io::stdout();
        loop {
            match handle.block_on(rx.recv()) {
                Ok(bytes) => {
                    if stdout.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // stdin → PTY. stdin().read() is blocking on every platform; we cannot
    // cancel it cooperatively. That is acceptable: the wrapper exits when
    // the child does, and the OS reclaims the stdin handle along with the
    // process. We just need to make sure we never hold raw mode after that.
    let pty_for_stdin = pty.clone();
    let stdin_task = tokio::task::spawn_blocking(move || {
        let handle = tokio::runtime::Handle::current();
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    if handle.block_on(pty_for_stdin.write_all(&chunk)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Watch PTY exit. When the child exits, the reader broadcast closes,
    // which itself terminates stdout_task — but we still want to give the
    // user a clean return path even if their terminal happens to be wired
    // such that stdout_task lingers.
    tokio::select! {
        _ = pty_exit.wait_exit() => {}
        _ = stdout_task => {}
        // stdin_task only terminates when stdin closes (Ctrl+Z on Windows,
        // Ctrl+D on Unix). Most users let `claude` exit and the PTY-side
        // pumps wind us down first.
        _ = stdin_task => {}
    }
}
