# ADR 0003: Use xterm.js for terminal rendering on the phone

## Status

Accepted, 2026-05.

## Context

The phone web app needs to show what's in the user's terminal. Two ways:
(a) feed raw PTY bytes into xterm.js, which handles ANSI escapes, cursor,
scrollback; or (b) parse the PTY stream into structured data (messages,
tool calls, etc.) and render React components.

## Decision

Use xterm.js for the main view. Mobile-friendly affordances (ActionBar with
arrow/Esc/Tab buttons, command palette) sit *around* the xterm.js canvas, not
inside it.

## Consequences

- Visual output is 1:1 with desktop Claude Code, automatically.
- We avoid fragile parsing of ANSI escape sequences and Claude TUI structure.
- Native autocomplete (when typing `/`) just works — Claude renders the menu;
  xterm.js displays it; the user navigates with the arrow buttons in
  ActionBar.
- We can iterate on UI affordances without touching the rendering pipeline.
- Mobile UX has rough edges (e.g., text selection in xterm is finicky on
  touch); acceptable trade-off for v1.

## Alternatives considered

- **Custom React renderer**: would give a fully mobile-native UI but requires
  parsing Claude's TUI which is undocumented and unstable.
- **Hybrid (parse only known structures, fall back to xterm.js)**: complex,
  and breaks when Claude updates rendering.
