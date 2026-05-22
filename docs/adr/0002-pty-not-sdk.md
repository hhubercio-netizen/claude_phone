# ADR 0002: Wrap Claude via PTY instead of using the Claude Agent SDK

## Status

Accepted, 2026-05.

## Context

We need to make the user's existing Claude Code session reachable from a phone.
Two ways to integrate: (a) wrap `claude` as a child process inside a PTY,
or (b) re-implement the agent using the Claude Agent SDK and call our own
agent from the web.

## Decision

Wrap the existing `claude` binary in a PTY. The wrapper doesn't know what the
contents mean — it's a transparent byte pipe.

## Consequences

- We get every feature of Claude Code for free, including new ones added later.
- We get the exact UI on the phone (xterm.js renders the same bytes).
- We pay no cost when Claude Code updates: no SDK migrations.
- We can't easily build mobile-native UI for things like tool approvals —
  they show up as the same TUI prompts as on desktop.
- We can't intercept individual messages or tool calls structurally; we only
  see the rendered terminal stream.

## Alternatives considered

- **Use Claude Agent SDK + custom React UI**: full control over UX, but we'd
  be re-building Claude Code. Massive scope, and the user explicitly wants
  *their session*, not a clone.
- **Hook-based integration**: Claude Code has hooks (PreToolUse, etc.) but
  they don't give us full I/O control — only event notifications.
