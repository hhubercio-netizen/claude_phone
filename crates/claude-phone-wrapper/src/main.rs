use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

use claude_phone_wrapper::bridge::run_via_pty;
use claude_phone_wrapper::cli::Cli;
use claude_phone_wrapper::config::WrapperConfig;
use claude_phone_wrapper::gateway_client::{GatewayClient, GatewayClientConfig};
use claude_phone_wrapper::local_term::{self, RawModeGuard};
use claude_phone_wrapper::pty::PtySession;
use claude_phone_wrapper::rpc::{RpcServer, RpcState};
use claude_phone_wrapper::session::SessionState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // The wrapper drives the host terminal in raw mode (see RawModeGuard),
    // so anything we print to stdout/stderr would corrupt the claude TUI.
    // Redirect tracing to a per-user log file instead. The path is announced
    // on stderr *before* we go into raw mode so the user knows where to
    // look if something goes wrong.
    let log_path = init_file_logging()?;
    eprintln!("claude-phone log: {}", log_path.display());

    let config_path = cli
        .config
        .clone()
        .or_else(WrapperConfig::default_path)
        .context("no --config given and no default config path resolvable")?;
    tracing::info!(?config_path, "loading wrapper config");
    let config =
        WrapperConfig::load(&config_path).with_context(|| format!("loading {config_path:?}"))?;

    // Session state + pair trigger channel.
    let session = Arc::new(tokio::sync::Mutex::new(SessionState::default()));
    let (pair_tx, mut pair_rx) = mpsc::channel::<()>(4);

    // Ephemeral bearer for the local RPC server. Generated fresh each start
    // and propagated to the child's env (CLAUDE_PHONE_RPC_TOKEN). Without it
    // any process reaching 127.0.0.1 could mint a session token; with it the
    // only callers that authenticate are descendants of this wrapper that
    // inherited the env (i.e. the `claude` PTY child and `claude-phone-pair`
    // invoked from inside it).
    let rpc_auth = claude_phone_shared::ApiKey::generate();

    // Start the RPC server BEFORE spawning the PTY so we know the listening
    // URL ahead of time and can inject CLAUDE_PHONE_RPC_URL into the child's
    // env. Without it the `/phone` plugin inside `claude` cannot find us.
    let rpc_state = RpcState {
        session: session.clone(),
        public_url_base: config.public_url_base.clone(),
        pair_trigger: pair_tx,
        auth_token: rpc_auth.clone(),
    };
    let rpc = RpcServer::start_with_state(&config.rpc_bind, rpc_state)
        .await
        .context("starting wrapper RPC server")?;
    let rpc_url = rpc.url();
    tracing::info!(
        rpc_url = %rpc_url,
        "wrapper RPC listening — POST {rpc_url}/pair to begin pairing",
    );

    // Spawn the PTY so the child can boot while the user scans the QR.
    // If plugin_dir is configured, prepend `--plugin-dir <path>` to the
    // claude-side args so the `/phone` command is loaded into the session.
    // This is what makes the plugin available without a global install.
    let plugin_dir_str: Option<String> = config
        .plugin_dir
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let mut claude_args: Vec<&str> = Vec::with_capacity(cli.claude_args.len() + 2);
    if let Some(p) = plugin_dir_str.as_deref() {
        claude_args.push("--plugin-dir");
        claude_args.push(p);
    }
    claude_args.extend(cli.claude_args.iter().map(String::as_str));
    let (cols, rows) = terminal_size::terminal_size()
        .map(|(w, h)| (w.0, h.0))
        .unwrap_or((80, 24));
    tracing::info!(claude_bin = %cli.claude_bin, ?claude_args, cols, rows, "spawning PTY");
    let (pty, first_rx) = PtySession::spawn(
        &cli.claude_bin,
        &claude_args,
        cols,
        rows,
        &[
            ("CLAUDE_PHONE_RPC_URL", rpc_url.as_str()),
            ("CLAUDE_PHONE_RPC_TOKEN", rpc_auth.as_str()),
        ],
    )
    .with_context(|| format!("spawning PTY with {}", cli.claude_bin))?;
    let pty = Arc::new(pty);

    // Take over the host terminal. The guard restores cooked mode on drop,
    // whether we exit normally or via panic.
    let _raw = RawModeGuard::enable().context("enabling raw mode on host terminal")?;

    // Start local terminal pumps. local_term::run completes when the PTY
    // exits or stdin closes — at which point the whole wrapper should shut
    // down because there is nothing useful left to do.
    let pty_for_local = pty.clone();
    let local_handle = tokio::spawn(async move {
        local_term::run(pty_for_local, first_rx).await;
    });

    // Track the currently-active bridge so a new /pair can preempt it.
    let mut active: Option<(oneshot::Sender<()>, JoinHandle<()>)> = None;

    // Drive pair triggers and bridges. We additionally race against the
    // local terminal exiting so the wrapper shuts down cleanly when claude
    // quits.
    let mut local_handle = local_handle;
    loop {
        tokio::select! {
            _ = &mut local_handle => {
                tracing::info!("local terminal exited; shutting down");
                break;
            }
            pair = pair_rx.recv() => {
                match pair {
                    None => {
                        tracing::warn!("pair channel closed; exiting");
                        break;
                    }
                    Some(()) => {
                        let s = session.lock().await;
                        let Some(token) = s.token.clone() else {
                            tracing::warn!("pair triggered but no token in session; ignoring");
                            continue;
                        };
                        // NB: do NOT log public_url with ?fmt — it contains the
                        // session token inside the path and tracing::fmt would
                        // write it to wrapper.log in cleartext. We only confirm
                        // its presence; the URL itself is delivered to the user
                        // via the QR/pair output, not the log.
                        let has_public_url = s.public_url.is_some();
                        drop(s);
                        tracing::info!(has_public_url, "pair triggered; connecting to gateway");

                        if let Some((cancel, handle)) = active.take() {
                            tracing::info!("preempting previous bridge");
                            let _ = cancel.send(());
                            let _ = handle.await;
                        }

                        let api_key = config.api_key.clone();
                        let url = config.gateway_url.clone();
                        let pty_for_bridge = pty.clone();
                        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

                        let handle = tokio::spawn(async move {
                            let client = match GatewayClient::connect(GatewayClientConfig {
                                url,
                                api_key,
                                token,
                                cols,
                                rows,
                            })
                            .await
                            {
                                Ok(c) => c,
                                Err(e) => {
                                    tracing::error!(error = %e, "gateway connect failed");
                                    return;
                                }
                            };
                            tracing::info!(
                                session_id = %client.session_id(),
                                "gateway connected; bridging PTY",
                            );
                            if let Err(e) = run_via_pty(client, pty_for_bridge, cancel_rx).await {
                                tracing::error!(error = %e, "bridge ended with error");
                            } else {
                                tracing::info!("bridge ended cleanly");
                            }
                        });

                        active = Some((cancel_tx, handle));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Install a `tracing_subscriber::fmt` writer that points at a per-user log
/// file. Returns the path so we can show it to the user.
///
/// The writer is constructed as a closure that reopens the file per event —
/// `tracing_subscriber` requires its writer to implement `MakeWriter`, and
/// the `Fn() -> impl Write` blanket impl is the cleanest path without
/// pulling in `tracing-appender`. Log volume here is light (startup +
/// occasional pair events), so per-event open is fine.
fn init_file_logging() -> anyhow::Result<std::path::PathBuf> {
    let dir = directories::ProjectDirs::from("", "", "claude-phone")
        .map(|d| d.data_local_dir().to_owned())
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating log dir {dir:?}"))?;
    let path = dir.join("wrapper.log");
    // 0o600 (Unix) so a multi-user host can't read the wrapper log. The
    // log carries peer IPs, RPC URL, error contexts — nothing as sensitive
    // as the api_key, but enough to fingerprint a session. The same mode
    // is applied to BOTH the probe open below and the writer factory so
    // a rotated file (deleted/recreated mid-process) keeps the same
    // restrictive permissions on Linux/macOS. Windows uses the default
    // user-profile ACL.
    fn restricted_open(path: &std::path::Path) -> std::io::Result<std::fs::File> {
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(path)
    }
    // Probe-open so we surface permission errors before init() (which can
    // only print to stderr we are about to take over).
    restricted_open(&path).with_context(|| format!("opening log file {path:?}"))?;
    let path_for_writer = path.clone();
    tracing_subscriber::fmt()
        .with_writer(move || {
            restricted_open(&path_for_writer)
                // TM-CODE.3: this fires inside the tracing writer factory.
                // If the log file can no longer be opened after daemonize
                // (disk full, permissions revoked) the wrapper has lost its
                // primary observability channel — fail loud rather than
                // silently swallow logs.
                .expect(
                    "re-opening wrapper.log after daemonize — \
                     unrecoverable; check disk and permissions",
                )
        })
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,claude_phone_wrapper=debug")),
        )
        .init();
    Ok(path)
}
