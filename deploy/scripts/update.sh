#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="${REPO_DIR:-/opt/claude-phone-src}"
INSTALL_DIR="${INSTALL_DIR:-/opt/claude-phone}"

if [[ "$(id -u)" -ne 0 ]]; then echo "run as root"; exit 1; fi

cd "$REPO_DIR"
git pull --ff-only

# Build new artifacts to a staging dir then atomically swap.
STAGE="$(mktemp -d)"
cargo build --release -p claude-phone-gateway
install -m 0755 target/release/claude-phone-gateway "$STAGE/"

npm ci
npm -w web run build
cp -r web/dist "$STAGE/web"

# Swap
mv "$INSTALL_DIR/bin/claude-phone-gateway" "$INSTALL_DIR/bin/claude-phone-gateway.old"
mv "$STAGE/claude-phone-gateway" "$INSTALL_DIR/bin/"
rsync -a --delete "$STAGE/web/" "$INSTALL_DIR/web/"

systemctl restart claude-phone-gateway

# Health check
sleep 2
if curl -sf http://127.0.0.1:8080/healthz > /dev/null; then
    echo "OK — new version up."
    rm -f "$INSTALL_DIR/bin/claude-phone-gateway.old"
else
    echo "Health check failed; rolling back."
    mv "$INSTALL_DIR/bin/claude-phone-gateway.old" "$INSTALL_DIR/bin/claude-phone-gateway"
    systemctl restart claude-phone-gateway
    exit 1
fi
