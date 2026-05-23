import React from 'react';
import ReactDOM from 'react-dom/client';
import './styles/globals.css';
import { App } from './App';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Register the PWA service worker. Done after render so the initial paint
// is never blocked by SW work, and only in production builds — the dev
// server serves modules that the cache-first SW would otherwise pin stale.
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch(() => {
      // Best-effort: a registration failure (private mode, restricted
      // context, etc.) shouldn't break the app.
    });
  });
}
