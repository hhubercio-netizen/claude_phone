#!/usr/bin/env bash
set -euo pipefail

# Initial deploy of claude-phone gateway on the home Ubuntu server.
# Assumes: Ubuntu 22.04+ (or similar), systemd, port 80/443 reachable or
# Cloudflare Tunnel set up.

REPO_DIR="${REPO_DIR:-/opt/claude-phone-src}"
INSTALL_DIR="${INSTALL_DIR:-/opt/claude-phone}"
ETC_DIR="${ETC_DIR:-/etc/claude-phone}"
USER="${CP_USER:-claude-phone}"
GROUP="${CP_GROUP:-claude-phone}"

require_root() {
    if [[ "$(id -u)" -ne 0 ]]; then
        echo "Run as root (sudo $0)" >&2
        exit 1
    fi
}

ensure_user() {
    if ! id "$USER" >/dev/null 2>&1; then
        useradd --system --no-create-home --shell /sbin/nologin "$USER"
    fi
}

ensure_dirs() {
    install -d -o "$USER" -g "$GROUP" "$INSTALL_DIR" "$INSTALL_DIR/bin" "$INSTALL_DIR/web"
    install -d "$ETC_DIR"
    install -d /var/lib/claude-phone
    chown "$USER:$GROUP" /var/lib/claude-phone
}

build_and_install() {
    cd "$REPO_DIR"
    cargo build --release -p claude-phone-gateway
    install -o "$USER" -g "$GROUP" -m 0755 \
        target/release/claude-phone-gateway "$INSTALL_DIR/bin/"

    npm ci
    npm -w web run build
    rsync -a --delete web/dist/ "$INSTALL_DIR/web/"
    chown -R "$USER:$GROUP" "$INSTALL_DIR/web"
}

install_config() {
    if [[ ! -f "$ETC_DIR/gateway.toml" ]]; then
        cat > "$ETC_DIR/gateway.toml" <<'EOF'
bind_addr = "127.0.0.1:8080"
static_dir = "/opt/claude-phone/web"
api_keys = [
    # Generate one with:  openssl rand -base64 32 | tr '+/' '-_' | tr -d '='
    # Paste here (exactly 43 chars).
]
session_idle_timeout_secs = 300
max_sessions = 32
log_format = "json"
EOF
        chmod 0640 "$ETC_DIR/gateway.toml"
        chown root:"$GROUP" "$ETC_DIR/gateway.toml"
        echo "Created $ETC_DIR/gateway.toml — edit it to add api_keys."
    fi
}

install_systemd() {
    install -m 0644 "$REPO_DIR/deploy/systemd/claude-phone-gateway.service" \
        /etc/systemd/system/
    systemctl daemon-reload
}

install_caddy_note() {
    if ! command -v caddy >/dev/null; then
        echo "Install Caddy: https://caddyserver.com/docs/install"
        echo "Then copy Caddyfile from deploy/caddy/Caddyfile to /etc/caddy/"
    fi
}

start_services() {
    systemctl enable --now claude-phone-gateway
    systemctl restart caddy 2>/dev/null || true
}

require_root
ensure_user
ensure_dirs
build_and_install
install_config
install_systemd
install_caddy_note
start_services

systemctl status --no-pager claude-phone-gateway || true
