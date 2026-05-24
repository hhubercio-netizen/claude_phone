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
session_idle_timeout_secs = 604800
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

# TM-INFRA.3 — sshd hardening drop-in. Idempotent: re-running just
# overwrites the file and reloads sshd. We `sshd -t` first so a broken
# config never reaches a reload and locks the operator out.
install_sshd_dropin() {
    if ! command -v sshd >/dev/null; then
        echo "sshd not installed; skipping TM-INFRA.3 drop-in" >&2
        return 0
    fi
    install -d -m 0755 /etc/ssh/sshd_config.d
    install -m 0644 "$REPO_DIR/deploy/sshd/99-claude-phone.conf" \
        /etc/ssh/sshd_config.d/99-claude-phone.conf
    if ! sshd -t; then
        echo "sshd -t failed AFTER installing drop-in; reverting" >&2
        rm -f /etc/ssh/sshd_config.d/99-claude-phone.conf
        return 1
    fi
    systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || true
}

# TM-INFRA.4 — fail2ban jails + claude-phone filter. Idempotent.
# Restart (not reload) because fail2ban reloads filter.d at startup
# only; a `reload` keeps the old failregex cached.
install_fail2ban() {
    if ! command -v fail2ban-server >/dev/null; then
        echo "fail2ban not installed; skipping TM-INFRA.4" >&2
        return 0
    fi
    install -d -m 0755 /etc/fail2ban/filter.d
    install -m 0644 "$REPO_DIR/deploy/fail2ban/jail.local" /etc/fail2ban/jail.local
    install -m 0644 "$REPO_DIR/deploy/fail2ban/filter.d/claude-phone.conf" \
        /etc/fail2ban/filter.d/claude-phone.conf
    systemctl enable --now fail2ban
    systemctl restart fail2ban
}

# TM-INFRA.5 — auditd watch rules for config, install dir, unit file,
# sshd drop-in. Idempotent. `augenrules --load` rebuilds /etc/audit/audit.rules
# from rules.d/* and signals auditd to reload — no service restart needed.
install_auditd() {
    if ! command -v augenrules >/dev/null; then
        echo "auditd not installed; skipping TM-INFRA.5" >&2
        return 0
    fi
    install -d -m 0755 /etc/audit/rules.d
    install -m 0640 "$REPO_DIR/deploy/auditd/claude-phone.rules" \
        /etc/audit/rules.d/claude-phone.rules
    augenrules --load
}

# TM-INFRA.9 — journald persistence drop-in. Idempotent.
# Restart journald so the drop-in takes effect; transient log lines
# emitted in the gap between stop and start get flushed by systemd.
install_journald() {
    install -d -m 0755 /etc/systemd/journald.conf.d
    install -m 0644 "$REPO_DIR/deploy/journald/99-claude-phone.conf" \
        /etc/systemd/journald.conf.d/99-claude-phone.conf
    install -d -m 2755 /var/log/journal
    systemctl restart systemd-journald
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
install_sshd_dropin
install_fail2ban
install_auditd
install_journald
start_services

systemctl status --no-pager claude-phone-gateway || true

# TM-TLS.6 / TM-TLS.7 — verify edge TLS posture (OCSP staple + testssl)
# before declaring the deploy complete. STRICT=1 means any HIGH/CRITICAL
# finding or missing staple aborts the deploy script. The presence and
# STRICT invocation are asserted by scripts/security_invariants.sh.
echo "[deploy] post-deploy TLS verify"
STRICT=1 bash "$REPO_DIR/deploy/scripts/post_deploy_verify.sh"
