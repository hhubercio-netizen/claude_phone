#!/usr/bin/env bash
# TM-INFRA.2 — apply ufw rules for the claude-phone host.
#
# Idempotent: re-running just re-asserts the desired state (reset +
# rebuild). The operator must populate these files before running:
#   /etc/claude-phone/ssh-allowlist   one IPv4/IPv6/CIDR per line
#   /etc/claude-phone/cf-ipv4          curl https://www.cloudflare.com/ips-v4
#   /etc/claude-phone/cf-ipv6          curl https://www.cloudflare.com/ips-v6
#
# Rationale for not committing CF IPs:
#   CF rotates IP ranges quarterly. Committing them creates a
#   maintenance burden and the stale-list regression can silently
#   lock out legitimate CF traffic. The operator fetches fresh per
#   apply; the runbook in deploy/cloudflare/README.md documents it.
#
# Default policy: deny incoming, allow outgoing. Explicit allow rules:
#   - SSH (22/tcp) from each line of ssh-allowlist
#   - HTTPS (443/tcp) from each CF range (v4 + v6)
#
# Comments on each rule encode the source — ufw status verbose then
# shows "ssh allow 1.2.3.4" / "cf 173.245.48.0/20" — critical when an
# operator debugs a lockout.

set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "Run as root (sudo $0)" >&2
    exit 1
fi

ALLOWLIST="${SSH_ALLOWLIST:-/etc/claude-phone/ssh-allowlist}"
CF4="${CF_IPV4:-/etc/claude-phone/cf-ipv4}"
CF6="${CF_IPV6:-/etc/claude-phone/cf-ipv6}"

# Fail loud, not silent, on missing pre-reqs. A typo in the path used
# to silently produce an empty firewall — i.e. allow nothing, lock out
# everyone. Empty-file check (-s) catches the curl-failed case too.
for f in "$ALLOWLIST" "$CF4" "$CF6"; do
    if [[ ! -s "$f" ]]; then
        echo "Missing or empty: $f" >&2
        echo "See deploy/cloudflare/README.md and SECURITY.md operator runbook." >&2
        exit 1
    fi
done

if ! command -v ufw >/dev/null; then
    echo "ufw not installed. apt install ufw" >&2
    exit 1
fi

ufw --force reset
ufw default deny incoming
ufw default allow outgoing

# SSH — operator-allowlisted sources only.
while IFS= read -r src; do
    # Skip blanks and comment lines.
    [[ -z "$src" || "$src" =~ ^# ]] && continue
    ufw allow proto tcp from "$src" to any port 22 comment "ssh allow $src"
done < "$ALLOWLIST"

# HTTPS — Cloudflare ingress only. Anything that bypasses CF (direct
# IP scan) is dropped at the kernel.
while IFS= read -r src; do
    [[ -z "$src" || "$src" =~ ^# ]] && continue
    ufw allow proto tcp from "$src" to any port 443 comment "cf $src"
done < "$CF4"

while IFS= read -r src; do
    [[ -z "$src" || "$src" =~ ^# ]] && continue
    ufw allow proto tcp from "$src" to any port 443 comment "cf $src"
done < "$CF6"

ufw --force enable
ufw status verbose
