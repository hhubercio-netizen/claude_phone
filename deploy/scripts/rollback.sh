#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${INSTALL_DIR:-/opt/claude-phone}"
if [[ "$(id -u)" -ne 0 ]]; then echo "run as root"; exit 1; fi

if [[ ! -f "$INSTALL_DIR/bin/claude-phone-gateway.old" ]]; then
    echo "No previous version saved."
    exit 1
fi

mv "$INSTALL_DIR/bin/claude-phone-gateway" \
   "$INSTALL_DIR/bin/claude-phone-gateway.broken"
mv "$INSTALL_DIR/bin/claude-phone-gateway.old" \
   "$INSTALL_DIR/bin/claude-phone-gateway"
systemctl restart claude-phone-gateway
