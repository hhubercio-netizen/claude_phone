# Architecture

> Top-level overview. For deeper dives see `docs/`.

## Goal

Continue a Claude Code session from your phone. Type `/phone` on the
desktop, scan a QR code, and a mobile-friendly web app gives you bidirectional
control of the *same* live session.

## Three layers

```
┌─────────────────────────────────────────┐
│  Developer's machine                    │
│                                         │
│  ┌──────────────────────────┐           │
│  │ Claude Code + plugin     │           │
│  │   /phone slash command   │           │
│  └────────┬─────────────────┘           │
│           │ HTTP POST                   │
│           ▼                             │
│  ┌──────────────────────────┐           │
│  │ claude-phone (Rust)      │           │
│  │  - PTY wrapper around    │           │
│  │    `claude`              │           │
│  │  - Local RPC server      │           │
│  │  - WSS client to gateway │           │
│  └────────┬─────────────────┘           │
└───────────┼─────────────────────────────┘
            │ WSS (api_key + token)
            ▼
┌───────────────────────────────────────┐
│  Home Ubuntu server                   │
│    Cloudflare → Caddy (TLS)           │
│    → claude-phone-gateway (Rust)      │
│       - Routes wrappers ↔ phones      │
│       - Serves React bundle           │
└───────────┬───────────────────────────┘
            │ WSS (tokenized URL)
            ▼
┌───────────────────────────────────────┐
│  Phone (web browser)                  │
│   claude-phone.pl/s/<token>           │
│   React + xterm.js + ActionBar        │
└───────────────────────────────────────┘
```

## Components

| Component | Tech | Source | Purpose |
|-----------|------|--------|---------|
| Plugin | Markdown + Bash | `plugin/` | Provides `/phone` slash command in Claude Code |
| Pair helper | Rust | `crates/claude-phone-pair/` | Called by the plugin; talks to wrapper's local RPC |
| Wrapper | Rust (tokio, axum, portable-pty, tungstenite) | `crates/claude-phone-wrapper/` | Spawns `claude` in a PTY, exposes local RPC, bridges PTY ↔ gateway WS |
| Shared types | Rust (serde) | `crates/claude-phone-shared/` | `SessionToken`, `ApiKey`, `ControlMessage` enum |
| Gateway | Rust (tokio, axum, dashmap) | `crates/claude-phone-gateway/` | WSS relay between wrapper and phone, serves static React |
| Frontend | React 18 + TypeScript + xterm.js + Tailwind | `web/` | Mobile-first terminal UI |

## Key design decisions

- **PTY wrapping, not Claude Agent SDK.** We treat `claude` as a black box so
  we get every feature for free and don't depend on SDK stability. See
  `docs/adr/0002-pty-not-sdk.md`.
- **Rust on both ends of the wire.** Wrapper and gateway need low latency and
  low memory; shared protocol types prevent drift. See
  `docs/adr/0001-rust-for-network.md`.
- **xterm.js for rendering**, with mobile affordances around it. The phone
  view is 1:1 with the desktop TUI. See `docs/adr/0003-xterm-for-rendering.md`.
- **256-bit session tokens** in the URL (capability URL). See `docs/security.md`.

## Data flow

End-to-end byte path when the user types on the phone:

```
Phone xterm.js
  → Phone WS (binary frame) to gateway via Cloudflare/Caddy
    → Gateway routes by session token to the wrapper's WS
      → Wrapper writes binary bytes to PTY stdin
        → claude (CLI) reads the bytes
```

And in reverse for Claude's output. Control messages (`phone_hello`,
`resize`, `peer_status`, `error`, `close`) are sent as JSON text frames; see
`docs/protocol.md` for the schema.

## Repository layout

```
claude-phone/
├── Cargo.toml                # workspace
├── rust-toolchain.toml       # pins 1.87.0
├── crates/
│   ├── claude-phone-shared/  # protocol + token types
│   ├── claude-phone-gateway/ # server-side relay
│   ├── claude-phone-wrapper/ # local PTY wrapper
│   └── claude-phone-pair/    # plugin helper
├── plugin/                   # Claude Code plugin (provides /phone)
├── web/                      # React frontend
├── deploy/                   # Caddyfile, systemd, Docker, scripts
└── docs/                     # API, protocol, security, ADRs, deployment
```

## Further reading

- `docs/protocol.md` — WS protocol message specs
- `docs/api.md` — gateway HTTP endpoints
- `docs/deployment.md` — how to deploy on a home server
- `docs/plugin-installation.md` — how to install the plugin on a dev machine
- `docs/security.md` — threat model and deferred hardening
- `docs/adr/` — architecture decision records
