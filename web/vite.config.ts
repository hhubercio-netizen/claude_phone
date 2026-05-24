import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    host: '0.0.0.0',
  },
  build: {
    outDir: 'dist',
    // Source maps disabled in prod: they exposed full TS sources under
    // /assets/*.js.map, handing reverse-engineers identifier names and
    // inline comments that describe the WS protocol shape.
    sourcemap: false,
  },
});
