#!/usr/bin/env bash
# TM-TLS.6 + TM-TLS.7 + TM-TLS.8 (and TM-TLS.1 belt-and-suspenders) —
# post-deploy verification of edge TLS posture.
#
# Run after every deploy and after any Cloudflare / DNS / Caddy change
# to confirm:
# - TM-TLS.1  edge negotiates TLS 1.3 and refuses TLS 1.2.
# - TM-TLS.6  OCSP stapling is active (openssl s_client -status).
# - TM-TLS.7  testssl.sh sees no HIGH / CRITICAL findings.
# - TM-TLS.8  Cloudflare SSL/TLS mode is "strict" (Full strict).
#
# Default (STRICT unset) soft-skips when the target is unreachable, so
# devs can run this against staging or during a planned outage without
# noise. Production deploy/scripts/deploy.sh invokes it with STRICT=1
# so any TLS regression aborts the deploy.
#
# Env:
#   CP_DOMAIN     domain to probe              default: claude-phone.pl
#   STRICT        1 → fail on any warning      default: 0 (soft)
#   TESTSSL_PATH  path to testssl.sh binary    default: $(command -v testssl.sh)
#   CF_API_TOKEN  Cloudflare token with Zone SSL+Certs read (TM-TLS.8)
#   CF_ZONE_ID    Cloudflare zone id for claude-phone.pl (TM-TLS.8)
#
# Exit 0 on pass (including soft skip), 1 on any failure under STRICT.

set -euo pipefail

DOMAIN="${CP_DOMAIN:-claude-phone.pl}"
STRICT="${STRICT:-0}"

is_strict() { [ "${STRICT}" = "1" ]; }

bail() {
    echo "FAIL: $*" >&2
    exit 1
}

soft_fail() {
    if is_strict; then
        bail "$@"
    fi
    echo "WARN (STRICT=0): $*" >&2
}

require() {
    command -v "$1" >/dev/null \
        || bail "required tool missing: $1"
}

require curl
require openssl

# --- TM-INFRA.10: gateway binds only to loopback -------------------------
#
# Runs first because it's host-local — does not need the public HTTPS
# surface to be up. A non-loopback bind is a critical configuration
# regression (e.g., bind_addr = "0.0.0.0:8080" typo would expose the
# gateway directly, bypassing Caddy TLS and the auth surface review).
#
# ss -tlnp output for the gateway looks like:
#   LISTEN 0  128  127.0.0.1:8080  0.0.0.0:*  users:(("claude-phone-g",pid=...))
#
# Soft-skip when ss is unavailable (probing from outside the deploy host)
# or when the gateway isn't visible (e.g., running in a different netns).

echo "[post_deploy] TM-INFRA.10 — gateway loopback binding"
if ! command -v ss >/dev/null; then
    soft_fail "ss(8) unavailable — TM-INFRA.10 binding check skipped (install iproute2)"
else
    binding=$(ss -tlnp 2>/dev/null \
        | grep -F claude-phone-gateway \
        | awk '{print $4}' \
        | head -n1 || true)
    if [ -z "${binding}" ]; then
        soft_fail "claude-phone-gateway not visible in ss -tlnp — TM-INFRA.10 not exercised"
    elif [[ "${binding}" =~ ^127\.0\.0\.1: ]] || [[ "${binding}" =~ ^\[::1\]: ]]; then
        echo "  OK: gateway bound to loopback only (${binding})"
    else
        soft_fail "gateway bound to non-loopback: ${binding} — TM-INFRA.10 regression (check gateway.toml bind_addr)"
    fi
fi

# --- Reachability gate ----------------------------------------------------

if ! curl -sf --max-time 10 "https://${DOMAIN}/healthz" >/dev/null; then
    soft_fail "https://${DOMAIN}/healthz unreachable — TLS checks skipped"
    exit 0
fi

echo "[post_deploy] target https://${DOMAIN} reachable; running TLS checks"

# --- TM-TLS.1: TLS 1.3 negotiated, TLS 1.2 refused -----------------------

echo "[post_deploy] TM-TLS.1 — TLS 1.3 negotiated, TLS 1.2 refused"
if ! openssl s_client \
        -connect "${DOMAIN}:443" -servername "${DOMAIN}" -tls1_3 \
        < /dev/null >/dev/null 2>&1; then
    soft_fail "TLS 1.3 handshake failed against ${DOMAIN}"
fi

if openssl s_client \
        -connect "${DOMAIN}:443" -servername "${DOMAIN}" \
        -tls1_2 -no_tls1_3 \
        < /dev/null >/dev/null 2>&1; then
    soft_fail "TLS 1.2 unexpectedly accepted at ${DOMAIN} — protocol downgrade"
else
    echo "  OK: TLS 1.2 refused"
fi

# --- TM-TLS.6: OCSP stapling ---------------------------------------------

echo "[post_deploy] TM-TLS.6 — OCSP stapling probe"
OCSP_OUT=$(openssl s_client \
    -connect "${DOMAIN}:443" -servername "${DOMAIN}" -status -tls1_3 \
    < /dev/null 2>/dev/null || true)

if grep -q "OCSP Response Status: successful" <<<"${OCSP_OUT}"; then
    echo "  OK: OCSP stapling active"
else
    soft_fail "OCSP stapling missing or unsuccessful at ${DOMAIN}"
fi

# --- TM-TLS.7: testssl.sh full scan --------------------------------------

TESTSSL=""
if [ -n "${TESTSSL_PATH:-}" ] && [ -x "${TESTSSL_PATH}" ]; then
    TESTSSL="${TESTSSL_PATH}"
elif command -v testssl.sh >/dev/null; then
    TESTSSL="testssl.sh"
fi

if [ -z "${TESTSSL}" ]; then
    soft_fail "testssl.sh not on PATH and TESTSSL_PATH unset — TM-TLS.7 not exercised. Install: git clone https://github.com/drwetter/testssl.sh /opt/testssl.sh && export TESTSSL_PATH=/opt/testssl.sh/testssl.sh"
else
    require jq

    echo "[post_deploy] TM-TLS.7 — testssl.sh full scan (~2 min)"
    TESTSSL_JSON=$(mktemp)
    trap 'rm -f "${TESTSSL_JSON}"' EXIT

    # --warnings off : skip interactive "are you sure?" prompts
    # --color 0      : strip ANSI for cleaner deploy logs
    # --quiet        : suppress banner
    # --jsonfile     : flat JSON array of findings
    if ! "${TESTSSL}" --quiet --warnings off --color 0 \
            --jsonfile "${TESTSSL_JSON}" \
            "https://${DOMAIN}/" >/dev/null 2>&1; then
        soft_fail "testssl.sh exited non-zero (network or invocation issue)"
    else
        HIGH_COUNT=$(jq '[.[] | select(.severity == "HIGH" or .severity == "CRITICAL")] | length' "${TESTSSL_JSON}")
        if [ "${HIGH_COUNT}" -gt 0 ]; then
            echo "FAIL: testssl.sh reported ${HIGH_COUNT} HIGH/CRITICAL finding(s):"
            jq '.[] | select(.severity == "HIGH" or .severity == "CRITICAL") | {id, severity, finding}' "${TESTSSL_JSON}"
            soft_fail "testssl.sh ${HIGH_COUNT} HIGH/CRITICAL — see above"
        else
            echo "  OK: testssl.sh — no HIGH/CRITICAL"
        fi
    fi
fi

# --- TM-TLS.8: Cloudflare TLS mode = Full (strict) -----------------------
#
# Asserts the CF zone setting is `strict` (not `flexible`, `full`, or `off`).
# Flexible would let an attacker who can reach the origin bypass TLS entirely;
# Full (without strict) accepts a self-signed origin cert and so loses MITM
# protection between CF and the home server. Only `strict` matches the design.

if [ -z "${CF_API_TOKEN:-}" ]; then
    soft_fail "CF_API_TOKEN unset — TM-TLS.8 (CF TLS mode) not exercised. See deploy/cloudflare/README.md."
elif [ -z "${CF_ZONE_ID:-}" ]; then
    soft_fail "CF_API_TOKEN set but CF_ZONE_ID missing — TM-TLS.8 cannot be verified"
else
    require jq

    echo "[post_deploy] TM-TLS.8 — Cloudflare TLS mode"
    CF_RESP=$(curl -fsS --max-time 10 \
        -H "Authorization: Bearer ${CF_API_TOKEN}" \
        -H "Content-Type: application/json" \
        "https://api.cloudflare.com/client/v4/zones/${CF_ZONE_ID}/settings/ssl" \
        2>/dev/null || echo '{"success":false}')

    if ! jq -e '.success == true' <<<"${CF_RESP}" >/dev/null; then
        soft_fail "Cloudflare API call failed (token invalid? zone_id wrong?) — TM-TLS.8"
    else
        CF_MODE=$(jq -r '.result.value // empty' <<<"${CF_RESP}")
        case "${CF_MODE}" in
            strict)
                echo "  OK: Cloudflare TLS mode = strict (Full strict)"
                ;;
            full|flexible|off|"")
                soft_fail "Cloudflare TLS mode = '${CF_MODE:-empty}' (expected 'strict' for TM-TLS.8)"
                ;;
            *)
                soft_fail "Cloudflare TLS mode = '${CF_MODE}' — unrecognized, expected 'strict'"
                ;;
        esac
    fi
fi

echo "[post_deploy] checks complete for ${DOMAIN} (STRICT=${STRICT})"
