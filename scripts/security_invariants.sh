#!/usr/bin/env bash
# Aggregate security invariant sweeps. Runs every shell-based security
# gate the workspace has today, in dependency-free order, and fails on
# the first failure with the gate's own message.
#
# Owners by sub-spec:
# - 4.13 TM-LEAK.3: asymmetric WS guard sweep (this commit).
# - 4.7  (planned): secret-scan, env-allowlist drift, gitleaks wiring —
#                   will extend this script with additional sections.
#
# Intentionally a thin aggregator. Each individual gate is its own file
# under scripts/ so it can be invoked directly during local debugging.

set -euo pipefail
cd "$(dirname "$0")/.."

echo "[security_invariants] asymmetric WS guard sweep ..."
./scripts/asymmetric_guards.sh

# TM-TLS.4 — pin the CT-monitor cron line. Daily 06:00 UTC is what the
# spec promises; a rename or accidental edit silently reduces coverage,
# so the regression check is grep-on-the-literal-string.
echo "[security_invariants] CT monitor cron presence ..."
grep -qF "cron: '0 6 * * *'" .github/workflows/ct-monitor.yml \
    || { echo "MISSING ct-monitor cron '0 6 * * *' — TM-TLS.4 regressed"; exit 1; }

# TM-TLS.6 / TM-TLS.7 — post-deploy verify must exist and be invoked by
# deploy.sh in STRICT mode. A refactor that "forgets" to call this would
# silently ship a TLS regression past the deploy gate.
echo "[security_invariants] post-deploy TLS verify wiring ..."
[ -x deploy/scripts/post_deploy_verify.sh ] \
    || { echo "MISSING executable deploy/scripts/post_deploy_verify.sh — TM-TLS.6/.7"; exit 1; }
grep -qF 'STRICT=1 bash "$REPO_DIR/deploy/scripts/post_deploy_verify.sh"' deploy/scripts/deploy.sh \
    || { echo "MISSING STRICT=1 invocation of post_deploy_verify.sh in deploy.sh — TM-TLS.6/.7"; exit 1; }

# TM-TLS.8 — assert the CF TLS-mode check is present in post_deploy_verify.sh.
# The check is conditional on CF_API_TOKEN at runtime, but the *code path*
# must always exist; a rewrite that drops this block would silently lose
# coverage for Full-strict drift on the Cloudflare account.
grep -qF 'settings/ssl' deploy/scripts/post_deploy_verify.sh \
    || { echo "MISSING Cloudflare TLS-mode check in post_deploy_verify.sh — TM-TLS.8"; exit 1; }

# TM-INFRA.1 / .6 / .8 / .11 + TM-RATE.5 — systemd unit hardening must
# stay present. A line-presence grep beats a runtime check: it works on
# any platform and trips on any future refactor that silently drops a
# directive. Runtime verification is post_deploy_verify.sh's job.
echo "[security_invariants] systemd unit hardening directives ..."
UNIT=deploy/systemd/claude-phone-gateway.service
for directive in \
    "^NoNewPrivileges=true" \
    "^ProtectSystem=strict" \
    "^SystemCallFilter=@system-service" \
    "^SystemCallFilter=~@privileged @resources" \
    "^SystemCallErrorNumber=EPERM" \
    "^LimitNOFILE=8192" \
    "^MemoryMax=256M" \
    "^LimitCORE=0" \
    "^ReadOnlyPaths=/opt/claude-phone"
do
    grep -qE "${directive}" "${UNIT}" \
        || { echo "MISSING in ${UNIT}: ${directive} — TM-INFRA.1/.6/.8/.11 or TM-RATE.5"; exit 1; }
done

# TM-INFRA.3 — sshd drop-in must keep the brute-force-relevant directives
# present. Allows the operator-specific AllowUsers to live in a separate
# 98-* drop-in without breaking this check.
echo "[security_invariants] sshd hardening drop-in ..."
SSHD_DROPIN=deploy/sshd/99-claude-phone.conf
[ -f "${SSHD_DROPIN}" ] \
    || { echo "MISSING ${SSHD_DROPIN} — TM-INFRA.3"; exit 1; }
for directive in \
    "^PermitRootLogin no" \
    "^PasswordAuthentication no" \
    "^PubkeyAuthentication yes" \
    "^ChallengeResponseAuthentication no" \
    "^KbdInteractiveAuthentication no" \
    "^MaxAuthTries 3" \
    "^LoginGraceTime 30" \
    "^X11Forwarding no" \
    "^AllowAgentForwarding no" \
    "^AllowTcpForwarding no" \
    "^PermitUserEnvironment no" \
    "^PermitEmptyPasswords no"
do
    grep -qE "${directive}" "${SSHD_DROPIN}" \
        || { echo "MISSING in ${SSHD_DROPIN}: ${directive} — TM-INFRA.3"; exit 1; }
done

# TM-INFRA.3 — deploy.sh must wire the drop-in install. A refactor that
# drops the call silently skips ssh hardening on every future deploy.
grep -qF 'install_sshd_dropin' deploy/scripts/deploy.sh \
    || { echo "MISSING install_sshd_dropin wiring in deploy.sh — TM-INFRA.3"; exit 1; }

# TM-INFRA.4 — fail2ban templates + filter regression. Presence checks
# are unconditional; the fail2ban-regex assertion is soft on hosts
# without fail2ban (dev workstations) — CI has it via the deploy-scripts
# job's apt cache.
echo "[security_invariants] fail2ban templates + filter regression ..."
for f in \
    deploy/fail2ban/jail.local \
    deploy/fail2ban/filter.d/claude-phone.conf
do
    [ -f "${f}" ] \
        || { echo "MISSING ${f} — TM-INFRA.4"; exit 1; }
done
grep -qF 'install_fail2ban' deploy/scripts/deploy.sh \
    || { echo "MISSING install_fail2ban wiring in deploy.sh — TM-INFRA.4"; exit 1; }
# Pin the failregex line so a drift that drops the ip capture or the
# event tag fails the gate before fail2ban-regex would even run.
grep -qF '"event":"auth_failure".*"ip":"<HOST>"' \
    deploy/fail2ban/filter.d/claude-phone.conf \
    || { echo "MISSING auth_failure+ip failregex pattern — TM-INFRA.4"; exit 1; }
if command -v fail2ban-regex >/dev/null 2>&1; then
    ./scripts/fail2ban_filter_test.sh
else
    echo "  (fail2ban-regex unavailable on this host — skipping regex assert)"
fi

# TM-INFRA.5 — auditd rules file must exist and contain the four
# watch keys spec'd in 4.9 §2.5. Presence-only: live auditctl/augenrules
# verification is post_deploy_verify.sh's job on the deploy host.
echo "[security_invariants] auditd watch rules ..."
AUDITD_RULES=deploy/auditd/claude-phone.rules
[ -f "${AUDITD_RULES}" ] \
    || { echo "MISSING ${AUDITD_RULES} — TM-INFRA.5"; exit 1; }
for key in \
    "^-w /etc/claude-phone/ -p wa -k claude-phone-config" \
    "^-w /opt/claude-phone/ -p wa -k claude-phone-bin" \
    "^-w /etc/systemd/system/claude-phone-gateway.service -p wa -k claude-phone-unit" \
    "^-w /etc/ssh/sshd_config.d/99-claude-phone.conf -p wa -k claude-phone-sshd"
do
    grep -qE "${key}" "${AUDITD_RULES}" \
        || { echo "MISSING auditd rule in ${AUDITD_RULES}: ${key} — TM-INFRA.5"; exit 1; }
done
grep -qF 'install_auditd' deploy/scripts/deploy.sh \
    || { echo "MISSING install_auditd wiring in deploy.sh — TM-INFRA.5"; exit 1; }

# TM-INFRA.9 — journald persistence + size caps + syslog forwarding.
# Presence-only here; live verification (Storage=persistent active
# after restart) is post_deploy_verify.sh's job on the deploy host.
echo "[security_invariants] journald persistence drop-in ..."
JOURNALD_DROPIN=deploy/journald/99-claude-phone.conf
[ -f "${JOURNALD_DROPIN}" ] \
    || { echo "MISSING ${JOURNALD_DROPIN} — TM-INFRA.9"; exit 1; }
for directive in \
    "^Storage=persistent" \
    "^SystemMaxUse=512M" \
    "^SystemKeepFree=2G" \
    "^MaxRetentionSec=30day" \
    "^ForwardToSyslog=yes"
do
    grep -qE "${directive}" "${JOURNALD_DROPIN}" \
        || { echo "MISSING in ${JOURNALD_DROPIN}: ${directive} — TM-INFRA.9"; exit 1; }
done
grep -qF 'install_journald' deploy/scripts/deploy.sh \
    || { echo "MISSING install_journald wiring in deploy.sh — TM-INFRA.9"; exit 1; }

# TM-INFRA.7 — Cloudflare WAF rule documentation. Operator-applied, not
# automated (see deploy/cloudflare/README.md for the reason). The check
# is doc-presence + the three rule names — if the runbook drifts away
# from the contract the gate trips, forcing a docs review.
echo "[security_invariants] Cloudflare WAF runbook (TM-INFRA.7) ..."
CF_README=deploy/cloudflare/README.md
for marker in \
    "Custom WAF rules (TM-INFRA.7)" \
    "block-scan-paths" \
    "block-unknown-api" \
    "rate-wrapper"
do
    grep -qF "${marker}" "${CF_README}" \
        || { echo "MISSING in ${CF_README}: ${marker} — TM-INFRA.7"; exit 1; }
done

# TM-INFRA.10 — loopback-only binding check must remain wired into
# post_deploy_verify.sh. A refactor that drops the block would silently
# stop catching bind_addr regressions on every future deploy.
echo "[security_invariants] post-deploy loopback binding check (TM-INFRA.10) ..."
grep -qF 'TM-INFRA.10' deploy/scripts/post_deploy_verify.sh \
    || { echo "MISSING TM-INFRA.10 binding block in post_deploy_verify.sh"; exit 1; }
grep -qE 'ss -tlnp.*claude-phone-gateway|grep -F claude-phone-gateway' \
        deploy/scripts/post_deploy_verify.sh \
    || { echo "MISSING ss -tlnp claude-phone-gateway probe — TM-INFRA.10"; exit 1; }
grep -qE '127\\.0\\.0\\.1:|\\[::1\\]:' deploy/scripts/post_deploy_verify.sh \
    || { echo "MISSING loopback pattern in post_deploy_verify.sh — TM-INFRA.10"; exit 1; }

echo "[security_invariants] OK"
