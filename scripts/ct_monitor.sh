#!/usr/bin/env bash
# TM-TLS.4 — Certificate Transparency log monitoring.
#
# Polls crt.sh for certificates issued under `claude-phone.pl`, diffs
# the set of unique serial numbers against a cached baseline, and exits
# 1 on any novel serial. The wrapping GitHub Actions workflow opens (or
# appends to) a tracking issue on exit 1, so a misissued or rogue CA
# cert surfaces in the GitHub inbox the day after CT logs it.
#
# A novel serial is NOT automatically malicious — legitimate edge-cert
# rotations at Cloudflare add serials too. The Issue body prompts a
# manual cross-check against the CF dashboard before any reaction.
#
# Exit codes (workflow distinguishes them):
#   0 — baseline matches (or first run, seeded)
#   1 — novel serial(s) detected
#   2 — crt.sh unreachable after 3 retries (soft skip — do not alert)
# 127 — required tool missing on PATH (jq / curl)
#
# Env overrides (mostly for tests):
#   CT_DOMAIN     domain to query        default: claude-phone.pl
#   CT_BASELINE   baseline file path     default: .ct-baseline
#   CT_NEW        working file path      default: .ct-new

set -euo pipefail
cd "$(dirname "$0")/.."

DOMAIN="${CT_DOMAIN:-claude-phone.pl}"
BASELINE="${CT_BASELINE:-.ct-baseline}"
NEW_FILE="${CT_NEW:-.ct-new}"
RAW_FILE="${NEW_FILE}.raw"

command -v jq   >/dev/null || { echo "ct_monitor: jq required but not on PATH"; exit 127; }
command -v curl >/dev/null || { echo "ct_monitor: curl required but not on PATH"; exit 127; }

fetch() {
    local attempt=0
    while [ "${attempt}" -lt 3 ]; do
        attempt=$((attempt + 1))
        if curl -fsS --max-time 30 \
            "https://crt.sh/?q=${DOMAIN}&output=json" \
            -o "${RAW_FILE}"; then
            return 0
        fi
        echo "ct_monitor: crt.sh attempt ${attempt}/3 failed; backing off 5s"
        sleep 5
    done
    return 1
}

if ! fetch; then
    echo "ct_monitor: crt.sh unreachable after 3 attempts — skipping this run"
    rm -f "${RAW_FILE}"
    exit 2
fi

# crt.sh sometimes returns HTML on overload — guard the JSON shape.
if ! jq -e '. | type == "array"' "${RAW_FILE}" >/dev/null 2>&1; then
    echo "ct_monitor: crt.sh returned non-array response (likely HTML error) — skipping"
    rm -f "${RAW_FILE}"
    exit 2
fi

# Empty array is valid: no certs logged for this domain yet.
if [ "$(jq 'length' "${RAW_FILE}")" -eq 0 ]; then
    echo "ct_monitor: crt.sh returned empty for ${DOMAIN} — nothing to baseline"
    rm -f "${RAW_FILE}"
    exit 0
fi

jq -r '.[] | select(.serial_number != null) | .serial_number' "${RAW_FILE}" \
    | tr 'A-F' 'a-f' \
    | sort -u > "${NEW_FILE}"
rm -f "${RAW_FILE}"

if [ ! -f "${BASELINE}" ]; then
    SEED_COUNT=$(wc -l < "${NEW_FILE}")
    echo "ct_monitor: no baseline yet — seeding with ${SEED_COUNT} cert serial(s)"
    mv "${NEW_FILE}" "${BASELINE}"
    exit 0
fi

NOVEL=$(comm -23 "${NEW_FILE}" "${BASELINE}" || true)

if [ -z "${NOVEL}" ]; then
    TRACKED=$(wc -l < "${NEW_FILE}")
    echo "ct_monitor: no change (${TRACKED} cert serial(s) tracked for ${DOMAIN})"
    rm -f "${NEW_FILE}"
    exit 0
fi

echo "ct_monitor: NOVEL CERT SERIAL(S) detected for ${DOMAIN}:"
# shellcheck disable=SC2086 # NOVEL is newline-separated serials; word-split is intentional so each prints on its own line.
printf '  %s\n' ${NOVEL}
mv "${NEW_FILE}" "${BASELINE}"
exit 1
