import type { ITheme } from '@xterm/xterm';

// ANSI palette approximating Claude Code's default terminal scheme.
// Tuned for OLED-friendly contrast on mobile.
// Pure-black background with pure-white default foreground. The semantic
// ANSI palette (red/green/yellow/...) is preserved so claude's own colour
// cues (errors red, success green, hints yellow) still come through. The
// only forced change is the "black" slot: claude paints text with ANSI
// black inside coloured boxes, and on a #000000 page that text was
// invisible — lifted to a near-black grey so the glyphs survive.
export const claudeTheme: ITheme = {
  background: '#000000',
  foreground: '#ffffff',
  cursor: '#ffffff',
  cursorAccent: '#000000',
  selectionBackground: '#444444',
  selectionForeground: '#ffffff',
  black: '#1a1a1a',
  red: '#cf6679',
  green: '#7fb069',
  yellow: '#e0c97f',
  blue: '#7aa6d6',
  magenta: '#c08bd6',
  cyan: '#7fc8c8',
  white: '#ffffff',
  brightBlack: '#7a7a7a',
  brightRed: '#ff8d9b',
  brightGreen: '#a4d68a',
  brightYellow: '#f5dc94',
  brightBlue: '#9ec3eb',
  brightMagenta: '#d6a3e8',
  brightCyan: '#a3e0e0',
  brightWhite: '#ffffff',
};
