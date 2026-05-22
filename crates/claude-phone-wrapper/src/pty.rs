use std::io::{Read, Write};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn Child + Send + Sync>,
    reader_rx: mpsc::Receiver<Vec<u8>>,
}

impl PtySession {
    pub fn spawn(program: &str, args: &[&str], cols: u16, rows: u16) -> anyhow::Result<Self> {
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

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            _child: child,
            reader_rx: rx,
        })
    }

    pub async fn read(&mut self) -> Option<Vec<u8>> {
        self.reader_rx.recv().await
    }

    pub async fn write_all(&mut self, data: &[u8]) -> anyhow::Result<()> {
        let owned = data.to_vec();
        let writer = std::mem::replace(&mut self.writer, Box::new(std::io::sink()));
        let writer = spawn_blocking(move || -> anyhow::Result<Box<dyn Write + Send>> {
            let mut w = writer;
            w.write_all(&owned)?;
            w.flush()?;
            Ok(w)
        })
        .await??;
        self.writer = writer;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
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
