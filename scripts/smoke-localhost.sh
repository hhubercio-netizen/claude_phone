#!/usr/bin/env bash
# Localhost end-to-end smoke test for claude-phone.
#
# Spawns gateway + wrapper, triggers a pair, runs scripts/smoke-phone.mjs
# against bash inside the wrapper PTY. Exits 0 if a bash command round-trips
# from the simulated phone through the bridge.
#
# Requires: cargo, node, ws npm pkg installed in repo root (node_modules/ws).
# Writes:   gateway-dev.toml, wrapper-dev.toml (gitignored) in repo root.

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"

API_KEY="${CLAUDE_PHONE_DEV_API_KEY:-}"
if [[ -z "$API_KEY" ]]; then
  # Generate a 32-byte random key in base64url (matches ApiKey::parse).
  API_KEY="$(node -e "
    const b = require('crypto').randomBytes(32);
    process.stdout.write(b.toString('base64url'));
  ")"
fi

cat > gateway-dev.toml <<EOF
bind_addr = "127.0.0.1:8080"
static_dir = "web/dist"
api_keys = ["$API_KEY"]
session_idle_timeout_secs = 300
max_sessions = 4
log_format = "pretty"
EOF

cat > wrapper-dev.toml <<EOF
gateway_url = "ws://127.0.0.1:8080/api/wrapper"
api_key = "$API_KEY"
public_url_base = "http://127.0.0.1:8080"
rpc_bind = "127.0.0.1:0"
EOF

log() { echo "[smoke] $*" >&2; }

cleanup() {
  log "stopping background procs"
  [[ -n "${WRAPPER_PID:-}" ]] && kill "$WRAPPER_PID" 2>/dev/null || true
  [[ -n "${GATEWAY_PID:-}" ]] && kill "$GATEWAY_PID" 2>/dev/null || true
}
trap cleanup EXIT

log "starting gateway"
cargo run --quiet -p claude-phone-gateway -- --config gateway-dev.toml \
  > /tmp/claude-phone-gateway.log 2>&1 &
GATEWAY_PID=$!

# Wait for gateway healthz to come up (up to ~30s — first build dominates).
for i in $(seq 1 60); do
  if curl -fs http://127.0.0.1:8080/healthz > /dev/null 2>&1; then
    break
  fi
  sleep 0.5
done
if ! curl -fs http://127.0.0.1:8080/healthz > /dev/null 2>&1; then
  log "FAIL: gateway never came up"
  tail -20 /tmp/claude-phone-gateway.log >&2 || true
  exit 1
fi
log "gateway up"

log "starting wrapper with --claude-bin bash"
cargo run --quiet -p claude-phone-wrapper -- \
  --config wrapper-dev.toml --claude-bin bash \
  > /tmp/claude-phone-wrapper.log 2>&1 &
WRAPPER_PID=$!

# Wait for wrapper to print its RPC URL.
RPC_URL=""
for i in $(seq 1 60); do
  if grep -q "CLAUDE_PHONE_RPC_URL=" /tmp/claude-phone-wrapper.log 2>/dev/null; then
    RPC_URL="$(grep -oE 'CLAUDE_PHONE_RPC_URL=http://[^ ]+' /tmp/claude-phone-wrapper.log \
      | head -1 | cut -d= -f2)"
    break
  fi
  sleep 0.5
done
if [[ -z "$RPC_URL" ]]; then
  log "FAIL: wrapper RPC never started"
  tail -20 /tmp/claude-phone-wrapper.log >&2 || true
  exit 1
fi
log "wrapper RPC at $RPC_URL"

log "triggering pair"
TOKEN="$(curl -fsX POST "$RPC_URL/pair" \
  | node -e "
    let d = '';
    process.stdin.on('data', c => d += c);
    process.stdin.on('end', () => {
      const o = JSON.parse(d);
      process.stdout.write(o.token);
    });
  ")"
log "got token (length=${#TOKEN})"

log "running scripts/smoke-phone.mjs"
PHONE_TOKEN="$TOKEN" node scripts/smoke-phone.mjs
log "smoke-phone OK"

log "running scripts/smoke-reconnect.mjs (sticky session)"
PHONE_TOKEN="$TOKEN" node scripts/smoke-reconnect.mjs
log "OK — end-to-end smoke + reconnect passed"
