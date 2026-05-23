use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::spawn_blocking;

/// A wrapped child process running inside a PTY.
///
/// The PTY's output is fan-out via a `tokio::sync::broadcast` channel so the
/// local terminal (when the wrapper runs interactively) AND the gateway
/// bridge (when a phone is paired) can both observe the same byte stream.
/// Writes are funnelled through a single `mpsc` channel that is drained on a
/// background blocking thread, so multiple async writers can call `write_all`
/// concurrently without external synchronisation.
pub struct PtySession {
    // Behind a Mutex only because `MasterPty: !Sync` — `resize()` is rare,
    // so contention is irrelevant.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer_tx: mpsc::Sender<Vec<u8>>,
    reader_bcast: broadcast::Sender<Vec<u8>>,
    /// Sticky exit signal flipped to `true` by the child-wait watcher when
    /// the wrapped process terminates (and as a fallback by the reader task
    /// on PTY EOF). Sticky semantics matter: late `wait_exit()` callers must
    /// still observe an already-occurred exit, which `watch` provides via
    /// `borrow()` but `Notify::notify_waiters` does not (it has no stored
    /// permit, so a notify with zero waiters is lost).
    exit_tx: watch::Sender<bool>,
    // Killer is retained so PtySession::drop terminates the child even if it
    // is still running. The owning `Child` lives on the wait-watcher thread
    // (see `spawn`), which calls `wait()` on it and signals `exit`. On Windows
    // ConPTY, the master reader does not necessarily EOF when the child
    // exits, so we cannot rely solely on the reader thread's EOF to fire
    // `exit` — the wait-watcher is the primary signal.
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Ok(mut k) = self.killer.lock() {
            let _ = k.kill();
        }
    }
}

impl PtySession {
    /// Spawn `program` with `args` inside a fresh PTY sized to `cols x rows`.
    ///
    /// `extra_env` is applied AFTER the inherited environment is filtered by
    /// [`env_is_forwardable`] — it is the only path by which `CLAUDE_PHONE_*`
    /// vars reach the child, and we use it to inject `CLAUDE_PHONE_RPC_URL`.
    ///
    /// Returns the session plus the **first broadcast subscription**. Holding
    /// this receiver before any other code runs guarantees no early child
    /// output is dropped on the floor — late subscribers (e.g. the gateway
    /// bridge after `/pair`) join the stream from the current head.
    pub fn spawn(
        program: &str,
        args: &[&str],
        cols: u16,
        rows: u16,
        extra_env: &[(&str, &str)],
    ) -> anyhow::Result<(Self, broadcast::Receiver<Vec<u8>>)> {
        let pty_sys = native_pty_system();
        let pair = pty_sys.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(program);
        for a in args {
            cmd.arg(*a);
        }
        cmd.env("TERM", "xterm-256color");
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        // Forward only safe environment variables. Copying *all* of std::env::vars()
        // leaks claude-phone secrets (CLAUDE_PHONE_API_KEY, CLAUDE_PHONE_CONFIG, etc.)
        // and any other API key the parent process happens to hold into the child
        // process environment, where it would be visible to plugins or subshells the
        // child spawns. We allowlist what `claude` actually needs.
        for (k, v) in std::env::vars() {
            if k == "TERM" {
                continue;
            }
            if !env_is_forwardable(&k) {
                continue;
            }
            cmd.env(k, v);
        }
        // Caller-supplied vars are injected last so they override any forwarded
        // value. This is the *only* path by which CLAUDE_PHONE_* reaches the
        // child — env_is_forwardable() blocks the prefix wholesale on purpose,
        // and we re-add a specific entry (CLAUDE_PHONE_RPC_URL) here.
        for (k, v) in extra_env {
            cmd.env(*k, *v);
        }

        let mut child = pair.slave.spawn_command(cmd)?;
        let killer = child.clone_killer();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Reader fan-out. 256 slots * up to 8 KiB each gives ~2 MiB of buffer
        // before a slow consumer starts losing data — plenty for an
        // interactive TUI.
        let (bcast_tx, first_rx) = broadcast::channel::<Vec<u8>>(256);
        let bcast_for_reader = bcast_tx.clone();
        let (exit_tx, _exit_rx) = watch::channel(false);
        let exit_tx_for_reader = exit_tx.clone();
        spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // `send` returns Err only when there are *no* live
                        // receivers. In that case the bytes are dropped on
                        // the floor, which is intentional: nobody is
                        // listening, nothing to do.
                        let _ = bcast_for_reader.send(buf[..n].to_vec());
                    }
                    Err(_) => break,
                }
            }
            let _ = exit_tx_for_reader.send(true);
        });

        // Writer pump. We funnel async writes into a blocking thread so any
        // number of tokio tasks can call `write_all` concurrently without an
        // explicit Mutex on the writer side. Backpressure is bounded by the
        // 128-slot channel.
        let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(128);
        spawn_blocking(move || {
            let mut writer = writer;
            while let Some(chunk) = writer_rx.blocking_recv() {
                if writer.write_all(&chunk).is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }
        });

        // Child-wait watcher. On Windows ConPTY the master reader does not
        // always EOF promptly after the child exits, so we cannot rely on
        // the reader thread's EOF path alone — this watcher is the
        // authoritative signal for `wait_exit`.
        let exit_tx_for_child = exit_tx.clone();
        spawn_blocking(move || {
            let _ = child.wait();
            let _ = exit_tx_for_child.send(true);
        });

        Ok((
            Self {
                master: Mutex::new(pair.master),
                writer_tx,
                reader_bcast: bcast_tx,
                exit_tx,
                killer: Mutex::new(killer),
            },
            first_rx,
        ))
    }

    /// Add a new subscriber to the PTY's output. Subscribers see only data
    /// produced *after* they subscribe; for a complete view from t=0 use the
    /// receiver returned by `spawn()`.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.reader_bcast.subscribe()
    }

    /// Enqueue `data` to be written to the PTY. Returns Err only if the
    /// writer task has died (e.g. the PTY closed).
    pub async fn write_all(&self, data: &[u8]) -> anyhow::Result<()> {
        self.writer_tx
            .send(data.to_vec())
            .await
            .map_err(|_| anyhow::anyhow!("pty writer channel closed"))?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        let guard = self
            .master
            .lock()
            .map_err(|_| anyhow::anyhow!("pty master mutex poisoned"))?;
        guard.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Resolves when the child process has exited. Late callers observe
    /// an already-occurred exit immediately (sticky watch semantics).
    pub async fn wait_exit(&self) {
        let mut rx = self.exit_tx.subscribe();
        if *rx.borrow() {
            return;
        }
        let _ = rx.changed().await;
    }
}

/// True for environment variables that are safe to forward to the child
/// process spawned in the PTY (i.e. the `claude` CLI).
///
/// We refuse anything that looks like a claude-phone-internal secret
/// (CLAUDE_PHONE_*) so a rogue plugin inside `claude` cannot read the gateway
/// api key. We also block common credential prefixes that have nothing to do
/// with terminal behavior.
fn env_is_forwardable(name: &str) -> bool {
    // Block claude-phone-internal config / secrets.
    if name.starts_with("CLAUDE_PHONE_") {
        return false;
    }
    // Block obvious credential vars; the wrapper itself only needs them once
    // at startup, the child does not.
    const BLOCKED_PREFIXES: &[&str] = &[
        "AWS_",
        "GCP_",
        "AZURE_",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "NPM_TOKEN",
        "CARGO_REGISTRY_TOKEN",
    ];
    for p in BLOCKED_PREFIXES {
        if name == *p || name.starts_with(p) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::env_is_forwardable;

    #[test]
    fn forwards_standard_env() {
        assert!(env_is_forwardable("HOME"));
        assert!(env_is_forwardable("PATH"));
        assert!(env_is_forwardable("USER"));
        assert!(env_is_forwardable("LANG"));
        assert!(env_is_forwardable("ANTHROPIC_API_KEY")); // claude needs it
    }

    #[test]
    fn blocks_claude_phone_internals() {
        assert!(!env_is_forwardable("CLAUDE_PHONE_API_KEY"));
        assert!(!env_is_forwardable("CLAUDE_PHONE_CONFIG"));
        assert!(!env_is_forwardable("CLAUDE_PHONE_GATEWAY_URL"));
    }

    #[test]
    fn blocks_other_credentials() {
        assert!(!env_is_forwardable("AWS_ACCESS_KEY_ID"));
        assert!(!env_is_forwardable("AWS_SECRET_ACCESS_KEY"));
        assert!(!env_is_forwardable("GITHUB_TOKEN"));
        assert!(!env_is_forwardable("GH_TOKEN"));
        assert!(!env_is_forwardable("NPM_TOKEN"));
    }
}
