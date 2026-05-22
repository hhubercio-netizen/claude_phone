# claude-phone plugin

Adds the `/phone` slash command to Claude Code. When invoked, prints a QR code
that, when scanned, opens a mobile-friendly web view of the current Claude
session.

## Requirements

- `claude-phone` wrapper installed and used to start Claude (not the raw
  `claude` binary).
- `claude-phone-pair` helper installed in `$PATH`.

## Install

```bash
./install.sh
```

## Use

Inside Claude Code:

```
/phone
```

The QR code appears in the terminal. Scan with your phone. The URL is
single-use and expires when the wrapper exits.

## How it works

`/phone` invokes `claude-phone-pair` via the Bash tool. The helper reads
`$CLAUDE_PHONE_RPC_URL` (set by the wrapper) and does a `POST /pair` to the
local RPC. The wrapper generates a 256-bit token, opens a WSS connection to the
gateway, and returns the URL + QR.
