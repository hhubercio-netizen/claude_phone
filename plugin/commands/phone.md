---
description: Show QR code to continue this session on your phone
allowed-tools: Bash(claude-phone-pair)
---

Execute the bash command `claude-phone-pair` exactly ONCE. After the tool returns, your entire reply MUST be the literal stdout the tool produced — nothing before it, nothing after it, no code fences, no commentary, no repetition.

Critical rules:
- Do not run `claude-phone-pair` more than once per `/phone` invocation. The QR encodes a single-use token; re-running it invalidates the previous one and confuses the user.
- Do not paraphrase, redraw, or "show the QR again for clarity". Print it exactly once.
- Do not wrap the output in triple backticks or any markdown — the unicode block characters that draw the QR must reach the terminal directly so they render as a scannable code.
- Do not add a heading like "Here is your QR code" or a trailing "Scan this with your phone" — the tool's own output already includes a labelled URL underneath the QR.

If the command fails because `CLAUDE_PHONE_RPC_URL` is not set, your entire reply must be exactly: `Run \`claude-phone\` (not \`claude\`) so the pairing RPC is available, then try /phone again.`
