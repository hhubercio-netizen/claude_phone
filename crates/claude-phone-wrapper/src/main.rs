use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

use claude_phone_wrapper::bridge::run_via_locked;
use claude_phone_wrapper::cli::Cli;
use claude_phone_wrapper::config::WrapperConfig;
use claude_phone_wrapper::gateway_client::{GatewayClient, GatewayClientConfig};
use claude_phone_wrapper::pty::PtySession;
use claude_phone_wrapper::rpc::{RpcServer, RpcState};
use claude_phone_wrapper::session::SessionState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,claude_phone_wrapper=debug")),
        )
        .init();

    let config_path = cli
        .config
        .clone()
        .or_else(WrapperConfig::default_path)
        .context("no --config given and no default config path resolvable")?;
    tracing::info!(?config_path, "loading wrapper config");
    let config =
        WrapperConfig::load(&config_path).with_context(|| format!("loading {config_path:?}"))?;

    // Spawn the PTY up-front so the child can boot while the user scans the QR.
    let claude_args: Vec<&str> = cli.claude_args.iter().map(String::as_str).collect();
    let (cols, rows) = terminal_size::terminal_size()
        .map(|(w, h)| (w.0, h.0))
        .unwrap_or((80, 24));
    tracing::info!(claude_bin = %cli.claude_bin, ?claude_args, cols, rows, "spawning PTY");
    let pty = PtySession::spawn(&cli.claude_bin, &claude_args, cols, rows)
        .with_context(|| format!("spawning PTY with {}", cli.claude_bin))?;
    let pty = Arc::new(Mutex::new(pty));

    // Session state + pair trigger channel.
    let session = Arc::new(Mutex::new(SessionState::default()));
    let (pair_tx, mut pair_rx) = mpsc::channel::<()>(4);

    let rpc_state = RpcState {
        session: session.clone(),
        public_url_base: config.public_url_base.clone(),
        pair_trigger: pair_tx,
    };
    let rpc = RpcServer::start_with_state(&config.rpc_bind, rpc_state)
        .await
        .context("starting wrapper RPC server")?;
    let rpc_url = rpc.url();
    // Print to stdout (separate from tracing, so smoke tests can grep it).
    println!("CLAUDE_PHONE_RPC_URL={rpc_url}");
    tracing::info!(
        rpc_url = %rpc_url,
        "wrapper RPC listening — POST {rpc_url}/pair to begin pairing",
    );

    // Track the currently-active bridge so a new /pair can preempt it before
    // taking the PTY lock. Without this, a stale bridge from a previous
    // pairing would still hold the lock and the new bridge would deadlock.
    let mut active: Option<(oneshot::Sender<()>, JoinHandle<()>)> = None;

    // Wait for pair triggers and bridge each one. We allow re-pairing after
    // a bridge ends so the user can reconnect from the phone.
    loop {
        match pair_rx.recv().await {
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
                let public_url = s.public_url.clone();
                drop(s);
                tracing::info!(?public_url, "pair triggered; connecting to gateway");

                // Preempt any previous bridge so it drops the PTY lock before
                // we ask for it. Ignore send-errors: if the receiver was
                // already dropped the task is on its way out anyway.
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
                    let guard = pty_for_bridge.lock_owned().await;
                    if let Err(e) = run_via_locked(client, guard, cancel_rx).await {
                        tracing::error!(error = %e, "bridge ended with error");
                    } else {
                        tracing::info!("bridge ended cleanly");
                    }
                });

                active = Some((cancel_tx, handle));
            }
        }
    }
    Ok(())
}
