#!/usr/bin/env bash
# TM-LEAK.3 — assert WS route guards are either symmetric across
# /api/wrapper and /api/phone/:token, or explicitly opted out with a
# rationale anchored to a TM-CAT.N ID in this script.
#
# Exit 1 on any expected guard missing or any unexpected guard present.
# Intended to run in pre-commit and CI; well under 1 s on the current
# source tree.
#
# The script is intentionally simple grep, not AST-based. The patterns
# are stable strings; any future refactor that renames PONG_DEADLINE etc.
# trips the sweep and forces the rename to update this script — which is
# the point. Rename-and-forget is exactly the failure mode TM-LEAK.3
# catches.

set -euo pipefail
cd "$(dirname "$0")/.."

WRAPPER="crates/claude-phone-gateway/src/routes/wrapper_ws.rs"
PHONE="crates/claude-phone-gateway/src/routes/phone_ws.rs"

for f in "$WRAPPER" "$PHONE"; do
    [ -f "$f" ] || { echo "FATAL: $f not found — has the route layout changed?"; exit 1; }
done

# --- Guard category presence: both routes ---------------------------------

assert_in_both() {
    local pat="$1"; local why="$2"
    grep -qE "$pat" "$WRAPPER" || { echo "MISSING in $WRAPPER: $pat ($why)"; exit 1; }
    grep -qE "$pat" "$PHONE"   || { echo "MISSING in $PHONE: $pat ($why)";   exit 1; }
}

assert_in_both 'max_message_size\(MAX_WS_MESSAGE_BYTES\)' 'TM-WS.4'
assert_in_both 'max_frame_size\(MAX_WS_MESSAGE_BYTES\)'   'TM-WS.5'
assert_in_both 'MAX_WS_MESSAGE_BYTES: usize = 64 \* 1024' 'TM-WS.4/5'
assert_in_both 'SINK_SEND_TIMEOUT'                        'TM-RATE.6'
assert_in_both 'PONG_DEADLINE'                            'TM-RATE.7 / TM-WS.7'
assert_in_both 'ConnRateLimiter::new'                     'TM-RATE.3'
assert_in_both 'ConnectInfo<SocketAddr>'                  '4.2 logging / 4.6 rate'
assert_in_both 'tokio::time::interval\(std::time::Duration::from_secs\(30\)\)' 'TM-WS.6'
assert_in_both 'public_origin.as_deref'                   'TM-WS.1, .2 Origin check'

# --- Asymmetric (one route only) ------------------------------------------

# HELLO_TIMEOUT is wrapper-only by design (phone has no post-upgrade hello).
grep -q 'HELLO_TIMEOUT' "$WRAPPER" \
    || { echo "MISSING HELLO_TIMEOUT in $WRAPPER (TM-RATE.8)"; exit 1; }
if grep -q 'HELLO_TIMEOUT' "$PHONE"; then
    echo "UNEXPECTED HELLO_TIMEOUT in $PHONE — phone has no post-upgrade hello"
    exit 1
fi

# Token-length strict check is phone-only (wrapper has no path token).
grep -q 'token_str.len() != SessionToken::LEN' "$PHONE" \
    || { echo "MISSING token length check in $PHONE (TM-WS.11)"; exit 1; }
if grep -q 'token_str.len() != SessionToken::LEN' "$WRAPPER"; then
    echo "UNEXPECTED token length check in $WRAPPER — wrapper has no token in path"
    exit 1
fi

# Fail-closed-on-missing-Origin is phone-only.
# (Wrapper rationale: §1.3 of 2026-05-23-sec-4.13-websocket.md — CLI-client carveout)
grep -qE 'Some\(o\) if o == expected' "$PHONE" \
    || { echo "MISSING fail-closed missing-Origin guard in $PHONE (TM-WS.3)"; exit 1; }
if grep -qE 'Some\(o\) if o == expected' "$WRAPPER"; then
    echo "UNEXPECTED fail-closed missing-Origin guard in $WRAPPER — wrapper is CLI"
    exit 1
fi

# --- Forbidden: any WebSocketUpgrade method we don't want -----------------

if grep -qE 'with_compression|\.protocols\(' "$WRAPPER" "$PHONE"; then
    echo "FORBIDDEN: WS compression or subprotocol negotiation must remain off (TM-WS.8, .12)"
    exit 1
fi

echo "asymmetric guards: OK"
