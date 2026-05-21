# Claude Phone

Use your Claude Code session from your phone. Type `/phone` in Claude Code, scan
the QR code, continue the conversation on mobile.

## Quick start

1. Install the wrapper and plugin on your dev machine (see `docs/plugin-installation.md`).
2. Deploy the gateway on your server (see `docs/deployment.md`).
3. Run `claude-phone` instead of `claude`. Inside Claude, type `/phone`.

## Repository layout

- `crates/claude-phone-wrapper/` — local PTY wrapper (Rust)
- `crates/claude-phone-gateway/` — server-side relay (Rust)
- `crates/claude-phone-shared/` — shared protocol types (Rust)
- `crates/claude-phone-pair/` — small helper called by the plugin (Rust)
- `plugin/` — Claude Code plugin providing `/phone`
- `web/` — React frontend served from the gateway
- `deploy/` — deployment configs (Caddy, systemd, docker-compose)

## Documentation

See `docs/`.
