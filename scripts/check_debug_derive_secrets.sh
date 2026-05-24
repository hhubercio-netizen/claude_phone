#!/usr/bin/env bash
# TM-LEAK.2 / TM-TEST.5 — fail CI if a struct has an auto-derived Debug
# AND a field named api_key/token/session_token/password whose declared
# type is NOT one of the redacting wrappers (ApiKey, SessionToken).
#
# The auto-derive prints field values verbatim, which silently leaks the
# secret to wrapper.log / journald the moment someone adds a
# `tracing::debug!(?value, ...)` anywhere on the call path. Hand-writing
# Debug (or typing the field as ApiKey/SessionToken) is the only way to
# stay safe across refactors.
#
# Safe field-level marker: declaring the field as
#     foo: ApiKey
#     foo: SessionToken
#     foo: Option<ApiKey>
#     foo: Option<SessionToken>
# exempts the struct (the redacting Debug on the wrapper takes over).
#
# Safe block-level marker:
#     // TM-LEAK.2: safe — <reason>
# anywhere inside the struct body. Use sparingly; the preferred fix is
# to wrap the field in a redacting type or hand-implement Debug.
#
# Exit 0 on pass, 1 on any unsafe match.

set -euo pipefail
cd "$(dirname "$0")/.."

# Scope is crates/*/src/. Tests are out of scope — Debug there is for
# assert_eq! diagnostics on fixture values, not for production logs.
files=$(find crates -path '*/src/*.rs' 2>/dev/null \
        | grep -v -E '(/target/|/\.cargo/)' || true)
[ -z "${files}" ] && { echo "check_debug_derive_secrets: no source files found"; exit 0; }

fail=0
for file in ${files}; do
    awk -v F="${file}" '
    BEGIN { in_block = 0; brace_depth = 0; start = 0; derive_seen = 0; block = "" }
    {
        # Capture a #[derive(...Debug...)] attribute. We then look for the
        # immediately-following struct declaration. The attribute and the
        # struct keyword can be separated by other attributes or doc
        # comments; track derive_seen and reset on each non-attribute,
        # non-comment, non-blank, non-struct line.
        # Two cases:
        #   #[derive(Debug, ...)] / #[derive(Debug)]   — Debug is first
        #   #[derive(X, Debug, ...)] / #[derive(X, Debug)] — Debug is not first
        # Single look-behind char class like `[, (]Debug` cannot express both,
        # because the leading `(` is already consumed by the literal `\(`.
        if (in_block == 0 \
            && $0 ~ /#\[derive\(/ \
            && ($0 ~ /#\[derive\(Debug[, )]/ || $0 ~ /#\[derive\([^)]*[, ]Debug[, )]/)) {
            derive_seen = NR
            next
        }
        # Strip line for classification.
        line_trim = $0
        sub(/^[ \t]+/, "", line_trim)

        if (derive_seen && in_block == 0) {
            # Tolerate intervening attributes / doc comments.
            if (line_trim ~ /^#\[/ || line_trim ~ /^\/\// || line_trim == "") next
            if (match(line_trim, /^(pub[ \t]+)?struct[ \t]+[A-Za-z_][A-Za-z0-9_]*/) != 0) {
                in_block = 1
                start = derive_seen
                block = $0
                opens  = gsub(/\{/, "{", $0)
                closes = gsub(/\}/, "}", $0)
                brace_depth = opens - closes
                # Single-line struct (e.g. `pub struct Foo {a:b}`) — rare.
                if (brace_depth <= 0 && /\}/) {
                    check_block(F, start, block)
                    in_block = 0
                    derive_seen = 0
                    brace_depth = 0
                    block = ""
                }
                next
            }
            # Anything else: the derive was for a non-struct (enum / fn).
            derive_seen = 0
            next
        }
        if (in_block) {
            block  = block "\n" $0
            opens  = gsub(/\{/, "{", $0)
            closes = gsub(/\}/, "}", $0)
            brace_depth += opens - closes
            if (brace_depth <= 0) {
                check_block(F, start, block)
                in_block = 0
                derive_seen = 0
                brace_depth = 0
                block = ""
            }
        }
    }
    function check_block(file, line, body,    is_unsafe) {
        # Look for a field named with a watch-listed identifier as a
        # whole word, possibly preceded by `pub`, followed by `:`.
        if (body !~ /[^A-Za-z0-9_](api_key|token|session_token|password)[ \t]*:/) return
        # Field-level safe: declared as a redacting wrapper type.
        if (body ~ /:[ \t]*(Option<[ \t]*)?ApiKey/)      return
        if (body ~ /:[ \t]*(Option<[ \t]*)?SessionToken/) return
        # Block-level safe: explicit override comment.
        if (body ~ /TM-LEAK\.2: safe/) return
        printf("%s:%d: TM-LEAK.2 — derive(Debug) on struct with bare secret-named field:\n", file, line) > "/dev/stderr"
        print body > "/dev/stderr"
        print "---" > "/dev/stderr"
        exit_code = 1
    }
    END { exit (exit_code ? 1 : 0) }
    ' "${file}" || fail=1
done

if [ "${fail}" -ne 0 ]; then
    echo "" >&2
    echo "FAIL: derive(Debug) on struct(s) with bare secret-named field (TM-LEAK.2)." >&2
    echo "Fix by one of:" >&2
    echo "  1. Type the field as ApiKey or SessionToken (redacting Debug)." >&2
    echo "  2. Hand-implement std::fmt::Debug to redact the field." >&2
    echo "  3. Drop Debug from the derive list if it is never used." >&2
    echo "  4. Add '// TM-LEAK.2: safe — <reason>' inside the struct body" >&2
    echo "     if the field genuinely is not a secret (rare; prefer 1–3)." >&2
    exit 1
fi

echo "check_debug_derive_secrets: OK"
