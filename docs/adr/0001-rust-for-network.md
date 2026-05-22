# ADR 0001: Rust for the network components

## Status

Accepted, 2026-05.

## Context

The wrapper and gateway sit in the hot path: every byte of PTY output and every
keystroke flows through them. We want sub-50ms median round-trip when the user
is on the same physical city as the server.

We also want low memory footprint on the home server (which runs other things)
and predictable behaviour under load (e.g., a phone that disconnects abruptly
shouldn't leak a goroutine/promise).

## Decision

Implement both wrapper and gateway in Rust on top of Tokio. Use `axum` for HTTP
and `tokio-tungstenite` for the WebSocket client side. PTY handled by
`portable-pty`.

## Consequences

- Slightly higher initial development cost vs Node/Python.
- Excellent runtime: ~5–10 MB RSS for the gateway, single-digit ms latency
  for in-memory routing.
- Strong type system prevents protocol drift between wrapper and gateway
  (shared crate enforces this).
- Cross-platform binary (wrapper runs on macOS/Linux/Windows via portable-pty).

## Alternatives considered

- **Node.js**: easier ecosystem for WS + xterm.js on the server, but PTY
  handling is fragile (`node-pty`) and we lose the strong typing across
  wrapper↔gateway.
- **Go**: similar performance to Rust, simpler syntax, but no shared types
  with the frontend either way, and PTY handling not as polished as
  `portable-pty`.
- **Python**: would not meet latency/memory targets.
