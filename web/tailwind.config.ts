import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // Claude Code TUI palette (extracted by hand)
        claude: {
          bg: '#0a0a0a',
          fg: '#e6e6e6',
          accent: '#ce8c5a',         // orange-ish from Claude branding
          muted: '#6e6e6e',
          panelBg: '#141414',
          panelBorder: '#2a2a2a',
          inputBg: '#1c1c1c',
          ok: '#7fb069',
          err: '#cf6679',
        },
      },
      fontFamily: {
        mono: ['"JetBrains Mono"', 'Menlo', 'Consolas', 'monospace'],
        sans: ['system-ui', '-apple-system', 'sans-serif'],
      },
    },
  },
  plugins: [],
} satisfies Config;
