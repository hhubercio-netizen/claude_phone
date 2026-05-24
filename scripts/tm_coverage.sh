#!/usr/bin/env bash
# tm_coverage.sh — TM-LEAK.4 bidirectional TM-ID coverage gate.
#
# TM-LEAK.4 spec text (threat-model.md §13): "every TM-CAT.N appears as
# comment in code or test". This gate enforces that contract on every
# push.
#
# Forward (catalog → references): every row in threat-model.md §13 must
# have at least one `TM-CAT.N` reference outside the catalog itself and
# the master security-hardening-design.md (which only enumerates IDs).
# Rows whose Status starts with TODO or DEFER, or contains "Accepted
# risk", are skipped — they represent mitigations that intentionally
# do not yet have code, so there is nothing to drift.
#
# Reverse (references → catalog): every `TM-CAT.N` found anywhere in
# tracked source/tests/scripts/deploy/web/docs/CI must exist as a row in
# the catalog. Catches typos and stale IDs after renames.
#
# Why both directions matter: a one-way check (catalog → code) would
# pass even after a typo that names no real mitigation (e.g. a renamed
# ID whose old form lingers as a comment), silently misleading future
# readers. A one-way check (code → catalog) would pass even after a
# GREEN mitigation drifts out of code, silently losing coverage.
# Bidirectional closes both gaps.

set -euo pipefail
cd "$(dirname "$0")/.."

CATALOG=docs/superpowers/specs/2026-05-23-threat-model.md
DESIGN=docs/superpowers/specs/2026-05-23-security-hardening-design.md

[ -f "$CATALOG" ] || { echo "MISSING $CATALOG — TM-LEAK.4"; exit 1; }
[ -f "$DESIGN" ]  || { echo "MISSING $DESIGN — TM-LEAK.4"; exit 1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# 1. Slice the catalog table out of §13. The section starts at
#    "## 13. Mitigation catalog" and ends at the next top-level
#    "## <digit>." heading.
awk '
    /^## 13\. /{ in_section=1; next }
    /^## [0-9]+\. /{ if (in_section) exit }
    in_section { print }
' "$CATALOG" > "$TMP/section.md"

# 2. Parse each catalog row into `id<TAB>skip_flag<TAB>status_text`.
#    Markdown rows are `| ID | DESC | STATUS |`. Some Status cells embed
#    an escaped `\|` inside backtick code (e.g. `ss -tlnp \| grep -F …`)
#    which fools a naive `-F'|'` split, so substitute a sentinel first
#    and restore after parsing.
sed 's/\\|/__TM_PIPE__/g' "$TMP/section.md" \
    | awk -F'|' '
        /^\| TM-[A-Z]+\.[0-9]+ /{
            id=$2; gsub(/^ +| +$/, "", id)
            status=$(NF-1); gsub(/^ +| +$/, "", status)
            token=status; sub(/[^A-Za-z].*/, "", token)
            skip=0
            if (token == "TODO" || token == "DEFER") skip=1
            if (index(status, "Accepted risk") > 0) skip=1
            gsub(/__TM_PIPE__/, "|", status)
            print id "\t" skip "\t" status
        }
    ' > "$TMP/catalog.tsv"

CATALOG_COUNT=$(wc -l < "$TMP/catalog.tsv")
if [ "$CATALOG_COUNT" -lt 50 ]; then
    echo "tm_coverage: catalog parser found only ${CATALOG_COUNT} rows — section parse likely broken"
    exit 1
fi

cut -f1 "$TMP/catalog.tsv" | sort -u > "$TMP/catalog_ids.txt"

# 3. Build the reference set: every TM-CAT.N anywhere outside the
#    catalog and the design spec. The design spec only enumerates IDs
#    in its own algorithm prose, so a hit there is not evidence the
#    mitigation lives in code.
grep -rEho 'TM-[A-Z]+\.[0-9]+' \
    --exclude='2026-05-23-threat-model.md' \
    --exclude='2026-05-23-security-hardening-design.md' \
    crates/ web/ deploy/ scripts/ docs/ .github/ 2>/dev/null \
    | sort -u > "$TMP/references.txt"

# 4. Forward check: catalog rows whose Status implies code exists must
#    have at least one outside reference.
MISSING_FWD=()
while IFS=$'\t' read -r id skip status; do
    [ "$skip" = "1" ] && continue
    if ! grep -qFx "$id" "$TMP/references.txt"; then
        MISSING_FWD+=("$id\t$status")
    fi
done < "$TMP/catalog.tsv"

# 5. Reverse check: every referenced ID must exist in the catalog.
MISSING_REV=()
while read -r id; do
    if ! grep -qFx "$id" "$TMP/catalog_ids.txt"; then
        MISSING_REV+=("$id")
    fi
done < "$TMP/references.txt"

# 6. Report and exit.
FAIL=0

if [ "${#MISSING_FWD[@]}" -ne 0 ]; then
    FAIL=1
    echo "tm_coverage (FORWARD / TM-LEAK.4): catalog IDs with NO outside reference"
    echo "  (status implies code exists but no \`// TM-CAT.N\` comment found)"
    for entry in "${MISSING_FWD[@]}"; do
        printf '  %b\n' "$entry"
    done
    echo
fi

if [ "${#MISSING_REV[@]}" -ne 0 ]; then
    FAIL=1
    echo "tm_coverage (REVERSE / TM-LEAK.4): IDs referenced in code but NOT in catalog"
    echo "  (likely a typo or stale ID after a rename)"
    for id in "${MISSING_REV[@]}"; do
        echo "  $id"
    done
    echo
fi

if [ "$FAIL" -ne 0 ]; then
    echo "tm_coverage: FAIL — fix listed gaps before merging."
    exit 1
fi

echo "tm_coverage: OK (${CATALOG_COUNT} catalog rows checked bidirectionally)"
