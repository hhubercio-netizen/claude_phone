#!/usr/bin/env bash
set -euo pipefail

PLUGIN_NAME="claude-phone"
PLUGINS_DIR="${HOME}/.claude/plugins"
TARGET="${PLUGINS_DIR}/${PLUGIN_NAME}"

mkdir -p "${PLUGINS_DIR}"

if [[ -e "${TARGET}" ]]; then
    echo "Plugin already installed at ${TARGET}. Removing and reinstalling."
    rm -rf "${TARGET}"
fi

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cp -r "${SCRIPT_DIR}" "${TARGET}"

echo "Installed plugin to ${TARGET}"
echo
echo "Next steps:"
echo "  1) Build and install the wrapper:    cargo install --path crates/claude-phone-wrapper"
echo "  2) Build and install the pair tool:  cargo install --path crates/claude-phone-pair"
echo "  3) Create config at \${XDG_CONFIG_HOME:-~/.config}/claude-phone/config.toml"
echo "  4) Run \`claude-phone\` instead of \`claude\`."
echo "  5) Type /phone inside Claude Code to pair your phone."
