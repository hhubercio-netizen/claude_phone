import type { ITheme } from '@xterm/xterm';

// ANSI palette approximating Claude Code's default terminal scheme.
// Tuned for OLED-friendly contrast on mobile.
export const claudeTheme: ITheme = {
  background: '#000000',
  foreground: '#e6e6e6',
  cursor: '#ce8c5a',
  cursorAccent: '#000000',
  selectionBackground: '#3a3a3a',
  selectionForeground: '#ffffff',
  black: '#000000',
  red: '#cf6679',
  green: '#7fb069',
  yellow: '#e0c97f',
  blue: '#7aa6d6',
  magenta: '#c08bd6',
  cyan: '#7fc8c8',
  white: '#cccccc',
  brightBlack: '#5a5a5a',
  brightRed: '#ff8d9b',
  brightGreen: '#a4d68a',
  brightYellow: '#f5dc94',
  brightBlue: '#9ec3eb',
  brightMagenta: '#d6a3e8',
  brightCyan: '#a3e0e0',
  brightWhite: '#ffffff',
};
