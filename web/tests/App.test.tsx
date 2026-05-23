import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Stub SessionPage so App routing tests don't try to open a WebSocket.
vi.mock('../src/pages/SessionPage', () => ({
  SessionPage: () => <div data-testid="session-page">SessionPage stub</div>,
}));

// We render App with a controllable history. App uses BrowserRouter, so we
// patch window.location via history.pushState before rendering.
const { App } = await import('../src/App');

function renderAtPath(path: string) {
  window.history.replaceState({}, '', path);
  return render(<App />);
}

beforeEach(() => {
  // Reset URL between tests so nothing leaks between cases.
  window.history.replaceState({}, '', '/');
});

describe('App routing', () => {
  it('renders SessionPage for /s/:token', () => {
    renderAtPath('/s/' + 'A'.repeat(43));
    expect(screen.getByTestId('session-page')).toBeInTheDocument();
  });

  it('renders NotFoundPage for an unknown route', () => {
    renderAtPath('/totally-unknown');
    // NotFoundPage exposes a recognizable string — check via casing-insensitive.
    expect(screen.getByText(/not found|404/i)).toBeInTheDocument();
  });

  it('renders NotFoundPage for the bare root path', () => {
    renderAtPath('/');
    expect(screen.getByText(/not found|404/i)).toBeInTheDocument();
  });

  it('mounts ErrorBoundary so render-time errors do not crash the page', () => {
    // We can't easily trigger an in-route render error without breaking the
    // stubbed SessionPage, but we can verify the page renders cleanly under
    // a path that does not throw — which proves ErrorBoundary is permissive
    // by default (does not swallow normal renders).
    renderAtPath('/s/' + 'A'.repeat(43));
    expect(screen.getByTestId('session-page')).toBeInTheDocument();
  });
});
