//! HTTP serving loop with slow-loris defense.
//!
//! Wraps hyper-util's auto::Builder so we can set http1.header_read_timeout
//! (TM-RATE.9) — a knob axum::serve does not expose. Lives in a module of
//! its own so the integration tests (`tests/rate_limit.rs`) can drive the
//! exact same code path the binary uses; otherwise a test against
//! `axum::serve` would silently miss the timeout regression.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::Service;

// TM-RATE.9 — slow-loris defense on the HTTP upgrade phase.
// 10s is generous for a legitimate client (browser sends headers in one
// go after TCP handshake; even a 56k modem clears 8 KiB of headers in
// under 2s) and tight for an attacker: the holder of a connection can
// stall at most 10 seconds per slot before the server reclaims it. The
// per-IP HTTP cap (TM-RATE.1, 10 burst) then bounds how many slots one
// IP can stall in parallel. Public so tests can refer to it by symbol.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

// Graceful shutdown bounded window. Past this the kernel TCP reset is
// acceptable — operators get to restart the binary in finite time.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the HTTP server with production defaults. Convenience wrapper for
/// `run_with`. See [`HEADER_READ_TIMEOUT`] for the TM-RATE.9 reasoning.
pub async fn run(listener: TcpListener, app: Router, shutdown: impl Future<Output = ()>) {
    run_with(
        listener,
        app,
        shutdown,
        HEADER_READ_TIMEOUT,
        GRACEFUL_TIMEOUT,
    )
    .await
}

/// Run the HTTP server with explicit timeouts. Exposed as a test seam so
/// integration tests can verify the slow-loris guard fires (TM-RATE.9)
/// without each test sleeping for the full production 10-second window.
/// A test that uses a 200 ms `header_read_timeout` still exercises the
/// exact same code path; only the wall-clock budget shrinks.
pub async fn run_with(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()>,
    header_read_timeout: Duration,
    graceful_timeout: Duration,
) {
    // TM-RATE.1 — tower_governor's PeerIpKeyExtractor pulls the source IP
    // from `axum::extract::ConnectInfo<SocketAddr>` in request extensions.
    // Without this wrapper the extractor returns UnableToExtractKey for
    // every request, the GovernorLayer responds 500, and per-IP rate
    // limiting silently does nothing. `into_make_service_with_connect_info`
    // is axum's official way to inject ConnectInfo per accepted connection.
    let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();
    let graceful = GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);

    loop {
        let conn = tokio::select! {
            accept = listener.accept() => accept,
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received");
                break;
            }
        };

        let (stream, peer) = match conn {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                continue;
            }
        };

        // Pull a per-connection Router that has this peer's ConnectInfo
        // pre-inserted. `MakeService::call(peer)` is infallible for the
        // axum-derived service, so `.expect` is the right shape — a
        // future axum change that introduces a fallible path here would
        // be a breaking API change we'd notice at compile time.
        // TM-CODE.3: justified expect — axum's IntoMakeService is infallible.
        let app_for_conn = match make_service.call(peer).await {
            Ok(svc) => svc,
            Err(e) => {
                let _: std::convert::Infallible = e;
                continue;
            }
        };

        let io = TokioIo::new(stream);
        let svc = TowerToHyperService::new(app_for_conn);
        // hyper-util auto::Builder negotiates HTTP/1 vs HTTP/2; WS rides on
        // HTTP/1.1 so the http1 branch is where the timeout has to live.
        // The auto variant is what gives us an UpgradeableConnection that
        // implements GracefulConnection (the plain http1::UpgradeableConnection
        // does not — see hyper-util 0.1.20 src/server/graceful.rs).
        let mut builder = auto::Builder::new(TokioExecutor::new());
        // TM-RATE.9: bound header-read time on the HTTP/1 path. Hyper
        // requires an explicit Timer wired in or the timeout panics with
        // "timeout `header_read_timeout` set, but no timer set" at first
        // request — TokioTimer adapts tokio::time to hyper's Sleep trait.
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(header_read_timeout);
        // `serve_connection_with_upgrades` is mandatory for WebSocket —
        // axum's WS extractor relies on hyper's upgrade machinery.
        let serve = builder.serve_connection_with_upgrades(io, svc);
        let watched = graceful.watch(serve.into_owned());
        tokio::spawn(async move {
            if let Err(e) = watched.await {
                tracing::debug!(error = %e, "connection ended with error");
            }
        });
    }

    tokio::select! {
        _ = graceful.shutdown() => tracing::info!("graceful shutdown complete"),
        _ = tokio::time::sleep(graceful_timeout) => {
            tracing::warn!("graceful shutdown timeout, forcing exit");
        }
    }
}
