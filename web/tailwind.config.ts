import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // Claude Code TUI palette (extracted by hand)
        claude: {
          bg: '#000000',
          fg: '#ffffff',
          accent: '#ffffff',
          muted: '#cccccc',
          panelBg: '#000000',
          panelBorder: '#ffffff',
          inputBg: '#000000',
          ok: '#7fb069',              // ONLY the "paired" indicator
          err: '#ffffff',
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
