import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useWakeLock } from '../src/hooks/useWakeLock';

interface MockSentinel {
  released: boolean;
  release: ReturnType<typeof vi.fn>;
  addEventListener: (type: 'release', fn: () => void) => void;
  _listeners: Array<() => void>;
}

function mkSentinel(): MockSentinel {
  const s: MockSentinel = {
    released: false,
    release: vi.fn(async () => {
      s.released = true;
      s._listeners.forEach((fn) => fn());
    }),
    addEventListener: (_t, fn) => {
      s._listeners.push(fn);
    },
    _listeners: [],
  };
  return s;
}

let originalWakeLock: unknown;

beforeEach(() => {
  originalWakeLock = (navigator as unknown as { wakeLock?: unknown }).wakeLock;
});

afterEach(() => {
  (navigator as unknown as { wakeLock?: unknown }).wakeLock = originalWakeLock;
});

describe('useWakeLock', () => {
  it('no-ops when navigator.wakeLock is undefined', () => {
    (navigator as unknown as { wakeLock?: unknown }).wakeLock = undefined;
    // Render with active=true; absence of crash + clean unmount = pass.
    const { unmount } = renderHook(() => useWakeLock(true));
    unmount();
  });

  it('requests a screen lock when active', async () => {
    const request = vi.fn(async () => mkSentinel());
    (navigator as unknown as { wakeLock: unknown }).wakeLock = { request };
    renderHook(() => useWakeLock(true));
    // microtask flush
    await Promise.resolve();
    await Promise.resolve();
    expect(request).toHaveBeenCalledWith('screen');
  });

  it('does NOT request a lock when active=false', async () => {
    const request = vi.fn(async () => mkSentinel());
    (navigator as unknown as { wakeLock: unknown }).wakeLock = { request };
    renderHook(() => useWakeLock(false));
    await Promise.resolve();
    expect(request).not.toHaveBeenCalled();
  });

  it('releases the lock on unmount', async () => {
    const sentinel = mkSentinel();
    const request = vi.fn(async () => sentinel);
    (navigator as unknown as { wakeLock: unknown }).wakeLock = { request };
    const { unmount } = renderHook(() => useWakeLock(true));
    await Promise.resolve();
    await Promise.resolve();
    unmount();
    // release is async; give the microtask a chance
    await Promise.resolve();
    expect(sentinel.release).toHaveBeenCalled();
  });

  it('swallows rejection from request without throwing', async () => {
    const request = vi.fn(async () => {
      throw new Error('not allowed');
    });
    (navigator as unknown as { wakeLock: unknown }).wakeLock = { request };
    const { unmount } = renderHook(() => useWakeLock(true));
    await Promise.resolve();
    await Promise.resolve();
    // No throw = pass. Unmount cleanly.
    unmount();
  });
});
