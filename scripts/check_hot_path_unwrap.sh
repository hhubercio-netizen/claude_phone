#!/usr/bin/env bash
# TM-CODE.3 — CI gate that fails when `.unwrap()` or `panic!(` appear in
# production source (anything in `crates/*/src/**.rs` above the per-file
# `#[cfg(test)] mod tests { ... }` marker).
#
# Production hot paths must use `.expect("...")` accompanied by a
# `// TM-CODE.3` comment that explains why the call is infallible. The
# comment is what gives a future reviewer a chance to spot a regression
# (e.g. swapping a derive(Serialize) struct for a hand-written impl that
# can fail).
#
# Heuristic: for each file, the FIRST line matching `#[cfg(test)]` or
# `mod tests {` marks the start of the test island. Anything *above* it
# is considered production code. This matches the convention used across
# the workspace; if you ever inline test functions above that line, this
# script will catch them — by design.
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
    done < <(grep -nE '\.unwrap\(\)|\bpanic!\(' "$f" || true)
done < <(find "$WORKSPACE_ROOT/crates" -type f -path '*/src/*.rs')

if [ "$FAILED" -ne 0 ]; then
    cat >&2 <<EOF

ERROR: TM-CODE.3 — \`.unwrap()\` or \`panic!()\` found in production code.

Production hot paths must use \`.expect("... reason ...")\` with a
\`// TM-CODE.3\` reference comment that explains why the call is
infallible in practice. Tests-only code goes below the
\`#[cfg(test)] mod tests { ... }\` marker.

EOF
    exit 1
fi

echo "TM-CODE.3 OK: no production .unwrap()/panic! in workspace src/."
