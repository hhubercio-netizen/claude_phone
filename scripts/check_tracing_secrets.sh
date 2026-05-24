#!/usr/bin/env bash
# TM-LEAK.1 / TM-TEST.5 — fail CI if any tracing! call site logs a
# secret-named identifier without going through our redacting types.
#
# Heuristic regex (per 4.15 §2.5):
#   tracing::{info,warn,error,debug,trace}!  followed by an argument
#   list that mentions \b(api_key|token|session_token|password)\b
#
# Safe markers (the call is exempted if any of these appear inside the
# call's source span):
#   - redact            — explicit redaction helper invoked in-line
#   - ApiKey(           — value constructed via the redacting wrapper
#   - SessionToken(     — likewise
#   - TM-LEAK.1: safe   — author-asserted safe site with rationale
#
# The script is intentionally heuristic, not AST-based. False positives
# get an inline `// TM-LEAK.1: safe — <reason>` comment; the comment is
# the audit trail.
#
# Exit 0 on pass, 1 on any unsafe match.

set -euo pipefail
cd "$(dirname "$0")/.."

# Limit scope to crates/*/src and crates/*/tests. build.rs, examples,
# and benches are intentionally out of scope (different threat surface).
files=$(find crates -path '*/src/*.rs' -o -path '*/tests/*.rs' 2>/dev/null \
        | grep -v -E '(/target/|/\.cargo/)' || true)
[ -z "${files}" ] && { echo "check_tracing_secrets: no source files found"; exit 0; }

fail=0
for file in ${files}; do
    awk -v F="${file}" '
    BEGIN { in_call = 0; depth = 0; start = 0; block = "" }
    {
        # Detect the start of a tracing! macro invocation. Captures only
        # the canonical macros — tracing::info!, ::warn!, ::error!,
        # ::debug!, ::trace!. Skips info_span! (span construction; not a
        # log emit and field names are still typed).
        if (in_call == 0 && match($0, /tracing::(info|warn|error|debug|trace)!\(/) != 0) {
            in_call = 1
            start = NR
            block = $0
            # Paren depth from this line — number of "(" minus ")".
            opens  = gsub(/\(/, "(", $0)
            closes = gsub(/\)/, ")", $0)
            depth  = opens - closes
            if (depth <= 0) {
                check_block(F, start, block)
                in_call = 0
                depth = 0
                block = ""
            }
            next
        }
        if (in_call) {
            block  = block "\n" $0
            opens  = gsub(/\(/, "(", $0)
            closes = gsub(/\)/, ")", $0)
            depth += opens - closes
            if (depth <= 0) {
                check_block(F, start, block)
                in_call = 0
                depth = 0
                block = ""
            }
        }
    }
    function check_block(file, line, body,    is_unsafe) {
        # Body mentions a watch-listed identifier as a whole word.
        if (body !~ /[^A-Za-z0-9_](api_key|token|session_token|password)[^A-Za-z0-9_]/) return
        # Safe markers: any of these inside the block exempt the call.
        if (body ~ /redact/)              return
        if (body ~ /ApiKey\(/)            return
        if (body ~ /SessionToken\(/)      return
        if (body ~ /TM-LEAK\.1: safe/)    return
        printf("%s:%d: TM-LEAK.1 — tracing! call mentions secret-named identifier:\n", file, line) > "/dev/stderr"
        print body > "/dev/stderr"
        print "---" > "/dev/stderr"
        exit_code = 1
    }
    END { exit (exit_code ? 1 : 0) }
    ' "${file}" || fail=1
done

if [ "${fail}" -ne 0 ]; then
    echo "" >&2
    echo "FAIL: tracing! call site(s) may leak secrets (TM-LEAK.1)." >&2
    echo "Fix by routing the value through ApiKey/SessionToken (so the" >&2
    echo "redacting Debug runs), or — if the mention is a string literal" >&2
    echo "or otherwise unambiguous — add a same-line" >&2
    echo "    // TM-LEAK.1: safe — <reason>" >&2
    echo "comment to acknowledge the heuristic match." >&2
    exit 1
fi

echo "check_tracing_secrets: OK"
