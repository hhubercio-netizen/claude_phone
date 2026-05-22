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
        for (k, v) in std::env::vars() {
            if k == "TERM" {
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
