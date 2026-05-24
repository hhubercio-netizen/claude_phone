#!/usr/bin/env bash
# TM-INFRA.4 — assert the claude-phone filter matches the auth-failure
# log format defined by sub-spec 4.2. If 4.2's format ever changes
# without updating the filter, this test fails in CI before deploy.
#
# Exit codes:
#   0 — filter matches the auth_failure line AND does not match the
#       auth_success line
#   1 — filter regression (wrong match count or false positive)
#   2 — fail2ban-regex not installed; caller decides whether to fail
#       (CI: yes; dev workstation: warn and continue)

set -euo pipefail

if ! command -v fail2ban-regex >/dev/null; then
    echo "fail2ban-regex not installed; install with: apt install fail2ban" >&2
    exit 2
fi

FILTER="deploy/fail2ban/filter.d/claude-phone.conf"
if [[ ! -f "${FILTER}" ]]; then
    echo "MISSING filter ${FILTER}" >&2
    exit 1
fi

SAMPLE=$(mktemp)
trap 'rm -f "${SAMPLE}"' EXIT

# Two canned lines: one auth_failure (MUST match), one auth_success
# (MUST NOT match). Mirrors the JSON shape pinned by TM-AUTH.7.
cat > "${SAMPLE}" <<'EOF'
2026-05-23T12:34:56.000Z {"timestamp":"2026-05-23T12:34:56.000Z","level":"WARN","fields":{"event":"auth_failure","conn_id":"7bff935b1c7265ad","peer_ip":"203.0.113.42","reason":"invalid_api_key","route":"wrapper_ws"}}
2026-05-23T12:35:00.000Z {"timestamp":"2026-05-23T12:35:00.000Z","level":"INFO","fields":{"event":"auth_success","conn_id":"3acdef0123456789","peer_ip":"203.0.113.99","route":"wrapper_ws"}}
EOF

# --print-no-missed suppresses unmatched-line printing (we only care
# about the failregex match counts).
result=$(fail2ban-regex --print-no-missed "${SAMPLE}" "${FILTER}" 2>&1 || true)

if ! echo "${result}" | grep -qE "Failregex.*1 match"; then
    echo "Expected 1 match in sample; fail2ban-regex output:" >&2
    echo "${result}" >&2
    exit 1
fi

# False-positive guard: the auth_success line must not be picked up.
# fail2ban-regex prints matched IPs; success-line IP would surface as
# a regex capture if the filter were over-broad.
if echo "${result}" | grep -qE "203\.0\.113\.99"; then
    echo "ERROR: filter matched auth_success line (false positive)" >&2
    echo "${result}" >&2
    exit 1
fi

echo "fail2ban filter regression: OK"
