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

echo "[security_invariants] OK"
