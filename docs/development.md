# Development guide

Setting up a local dev environment for working on Claude Phone.

## Prerequisites

- Rust 1.87+ (pinned by `rust-toolchain.toml`; rustup will auto-install).
- Node.js 20+ and npm.
- Git.
- For full PTY tests: bash + standard Unix tools (Linux/macOS/WSL). PTY tests are
  skipped on Windows native — the wrapper still builds and runs there for
  development with `bash` from Git for Windows.

## Build everything

```bash
cargo build --workspace
npm install
npm -w web run build
```

## Run tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

cd web
npm test
npm run build
```

## End-to-end smoke test (without phone)

Use `bash` as a stand-in for `claude` to exercise the bridge without needing a
real Claude Code session.

### Quickest: one-shot script

```bash
./scripts/smoke-localhost.sh
```

Spawns gateway + wrapper, pairs once, runs `scripts/smoke-phone.mjs` which
simulates a phone-side WebSocket client, types `ls /; echo MARKER` into bash,
and asserts the marker echoes back through the bridge. Exits 0 on success,
non-zero with diagnostic logs on failure. Cleans up both background processes
on exit.

The script writes `gateway-dev.toml` and `wrapper-dev.toml` in the repo root
(both gitignored) with a freshly generated API key.

### Manual walkthrough

If you want to drive each piece by hand:

### 1. Generate a test API key

```bash
KEY=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=' | head -c 43)
echo "$KEY"
```

### 2. Start the gateway locally

Create `gateway-dev.toml`:

```toml
bind_addr = "127.0.0.1:8080"
static_dir = "web/dist"
api_keys = ["PASTE_KEY_FROM_STEP_1"]
session_idle_timeout_secs = 604800  # 7 days; resets on phone attach
max_sessions = 4
log_format = "pretty"
```

Then:

```bash
cargo run -p claude-phone-gateway -- --config gateway-dev.toml
```

Leave it running in this terminal.

### 3. Start the wrapper in another terminal

Create `~/.config/claude-phone/config.toml`:

```toml
gateway_url = "ws://127.0.0.1:8080/api/wrapper"
api_key = "SAME_KEY_AS_STEP_1"
public_url_base = "http://127.0.0.1:8080"
```

Then run:

```bash
cargo run -p claude-phone-wrapper -- --claude-bin bash
```

You should be dropped into an interactive bash prompt running inside the
wrapper. The wrapper's RPC URL is now in the `CLAUDE_PHONE_RPC_URL` env var
(visible in the bash session: `echo $CLAUDE_PHONE_RPC_URL`).

### 4. Trigger pairing

From a third terminal:

```bash
curl -s -X POST "${CLAUDE_PHONE_RPC_URL}/pair" | jq .
# Or if claude-phone-pair is installed:
claude-phone-pair
```

The response contains a URL like `http://127.0.0.1:8080/s/<43chars>`. Open it
in a browser (Chrome DevTools → toggle device toolbar → iPhone for mobile
preview).

### 5. Verify bidirectional bytes

- Type in the wrapper's bash terminal — characters should appear in the
  browser's xterm.js view.
- Tap the ActionBar's `↵` button (or type in the browser) — keystrokes go
  through the gateway to the wrapper's PTY and into bash.
- Type `ls -la`, `pwd`, `whoami` — output streams back to both sides.

This validates the full end-to-end pipeline. If it works with `bash`, it will
work with `claude`.

## Common dev workflows

- Quick rebuild + run gateway: `cargo run -p claude-phone-gateway -- --config gateway-dev.toml`
- Web dev server with hot reload: `npm -w web run dev` (listens on
  `http://0.0.0.0:5173/s/<token>` — useful for mobile testing on LAN).
- Watch all crates: `cargo watch -x 'check --workspace'` (requires
  `cargo install cargo-watch`).

## Code conventions

- Rust: `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass.
- Frontend: Prettier + ESLint (config lands in M5.1+; for now run
  `npm -w web run build` to catch type errors via `tsc`).
- Commit messages: conventional (`feat:`, `fix:`, `chore:`, `docs:`, `test:`,
  `ci:`, `refactor:`).
- Each task in the plan gets its own commit — see
  `C:\Users\mrzyg\.claude\plans\expressive-snuggling-parasol.md`.
