#!/usr/bin/env bash
# TM-CODE.5 — CI gate that fails if production code introduces an
# unbounded tokio channel (`mpsc::unbounded_channel`, `unbounded_send`).
#
# Unbounded channels are a slow-leak DoS vector: a producer that runs
# faster than the consumer fills the heap until the process is killed
# by the OOM killer. Every channel in the gateway and wrapper hot paths
# is bounded (see e.g. `tokio::sync::mpsc::channel(N)`); this script
# locks the invariant in so a future contributor can't quietly add an
# unbounded one.
#
# Tests are exempt — they use unbounded channels as cheap queues with
# fully-controlled producers, where the leak risk doesn't apply.
set -euo pipefail

FAILED=0
WORKSPACE_ROOT=$(cd "$(dirname "$0")/.." && pwd)

while IFS= read -r f; do
    test_marker=$(awk '
        /^[[:space:]]*#\[cfg\(test\)\]/ { print NR; exit }
        /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+tests?[[:space:]]*\{/ { print NR; exit }
    ' "$f")

    while IFS=: read -r lineno line; do
        if [ -z "$test_marker" ] || [ "$lineno" -lt "$test_marker" ]; then
            echo "$f:$lineno: $line"
            FAILED=1
        fi
    done < <(grep -nE 'unbounded_channel\b|unbounded_send\b' "$f" || true)
done < <(find "$WORKSPACE_ROOT/crates" -type f -path '*/src/*.rs')

if [ "$FAILED" -ne 0 ]; then
    cat >&2 <<EOF

ERROR: TM-CODE.5 — unbounded tokio channel found in production code.

Use \`tokio::sync::mpsc::channel(N)\` with an explicit capacity. The
capacity acts as natural back-pressure; an unbounded channel turns a
slow consumer into a heap leak that ends with the OOM killer.

EOF
    exit 1
fi

echo "TM-CODE.5 OK: no unbounded channels in workspace src/."
