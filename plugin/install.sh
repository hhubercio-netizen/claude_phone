#!/usr/bin/env bash
set -euo pipefail

# Modern Claude Code discovers plugins via `--plugin-dir <path>` (per-session)
# or marketplaces (persistent). The `claude-phone` wrapper auto-passes
# `--plugin-dir <path>` when `plugin_dir` is set in its config.toml, so the
# recommended install is simply to point the wrapper at this directory.

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

echo "Validating plugin manifest..."
claude plugin validate "${SCRIPT_DIR}"

cat <<EOF

Plugin path:  ${SCRIPT_DIR}

Next steps:
  1) Build and install the wrapper:  cargo install --path crates/claude-phone-wrapper
  2) Build and install the pair tool: cargo install --path crates/claude-phone-pair
  3) Create \${XDG_CONFIG_HOME:-~/.config}/claude-phone/config.toml with at
     minimum:

        gateway_url = "wss://<your-host>/api/wrapper"
        api_key     = "<43-char base64url key>"
        plugin_dir  = "${SCRIPT_DIR}"

  4) Run \`claude-phone\` instead of \`claude\`.
  5) Type /phone inside Claude Code to pair your phone.
EOF
