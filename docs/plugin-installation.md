# Installing the claude-phone plugin on your dev machine

The `/phone` command isn't built into Claude Code — it lives in a small plugin
that ships with this repository. Two binaries plus one plugin file need to be
installed.

## Prerequisites

- Working `claude` CLI (Claude Code).
- Rust toolchain (1.87+, pinned by `rust-toolchain.toml`).
- A wrapper config at `$XDG_CONFIG_HOME/claude-phone/config.toml` (Linux/macOS)
  or `%APPDATA%/claude-phone/config.toml` (Windows) containing your API key
  and gateway URL. See `docs/deployment.md` Step 4 for generating the key.

## One-time install

From the repo root:

```bash
# Build and install the wrapper binary (drops `claude-phone` into ~/.cargo/bin)
cargo install --path crates/claude-phone-wrapper

# Build and install the small helper the plugin uses
cargo install --path crates/claude-phone-pair

# Install the Claude Code plugin (copies plugin/ into ~/.claude/plugins/claude-phone)
bash plugin/install.sh
```

Verify both binaries are on PATH:

```bash
which claude-phone
which claude-phone-pair
```

## Wrapper config

Create `~/.config/claude-phone/config.toml`:

```toml
gateway_url = "wss://claude-phone.pl/api/wrapper"
api_key = "PASTE_43_CHAR_KEY"
public_url_base = "https://claude-phone.pl"
```

`public_url_base` is the prefix used in the QR-coded URL — typically the same
domain as `gateway_url` but with `https://` scheme.

## Smoke test

```bash
claude-phone --claude-bin bash
# Inside the bash prompt that comes up:
echo "hello"
# Now exit and try with claude itself:
claude-phone
# Inside claude:
/phone
```

The QR code should appear directly in the SSH/terminal output. Scan it with
your phone; the browser opens to `https://claude-phone.pl/s/<token>` and
within a second or two shows your live claude session.

## Troubleshooting

- **`/phone` reports "CLAUDE_PHONE_RPC_URL not set"** — you ran `claude`
  instead of `claude-phone`. The plugin needs the wrapper to be the parent
  process, because that's what holds the local RPC server.
- **Wrapper exits with `invalid_api_key`** — check the key in
  `gateway.toml` on the server matches the one in your local config.
- **Phone shows "Session not found"** — token expired (wrapper exited).
  Run `/phone` again to mint a fresh token.
- **WebSocket fails to upgrade behind Cloudflare** — confirm WebSockets are
  enabled in Cloudflare dashboard (Network → WebSockets: On).
