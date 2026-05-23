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

echo "[security_invariants] OK"
