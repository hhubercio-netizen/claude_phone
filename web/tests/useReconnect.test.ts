import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { MockWebSocket, installMockWebSocket } from './mock-ws';
import { useReconnectingWebSocket } from '../src/hooks/useReconnect';

const BACKOFF_MAX_MS = 16000;
const JITTER_MS = 250;

beforeEach(() => {
  vi.useFakeTimers();
  installMockWebSocket();
  MockWebSocket.reset();
  // Pin jitter so we can assert exact delays.
  vi.spyOn(Math, 'random').mockReturnValue(0);
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('useReconnectingWebSocket', () => {
  it('opens initial connection on mount', () => {
    renderHook(() => useReconnectingWebSocket('wss://example.com/x'));
    expect(MockWebSocket.instances.length).toBe(1);
  });

  it('schedules a reconnect after a close', () => {
    renderHook(() => useReconnectingWebSocket('wss://example.com/x'));
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    act(() => {
      MockWebSocket.last()!.simulateClose(1006, 'gone');
    });
    expect(MockWebSocket.instances.length).toBe(1);
    act(() => {
      vi.advanceTimersByTime(500 + JITTER_MS);
    });
    expect(MockWebSocket.instances.length).toBe(2);
  });

  it('uses exponential backoff for successive reconnects', () => {
    renderHook(() => useReconnectingWebSocket('wss://example.com/x'));
    // 1st close -> 500ms
    act(() => MockWebSocket.last()!.simulateClose());
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(MockWebSocket.instances.length).toBe(2);

    // 2nd close -> 1000ms
    act(() => MockWebSocket.last()!.simulateClose());
    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(MockWebSocket.instances.length).toBe(2);
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(MockWebSocket.instances.length).toBe(3);
  });

  it('caps backoff at the documented maximum', () => {
    renderHook(() => useReconnectingWebSocket('wss://example.com/x'));
    for (let i = 0; i < 10; i++) {
      act(() => MockWebSocket.last()!.simulateClose());
      act(() => {
        vi.advanceTimersByTime(BACKOFF_MAX_MS + JITTER_MS);
      });
    }
    // Last scheduled delay must not exceed the documented max + jitter.
    // We cannot read the delay directly, but ensure timers progress at the
    // capped rate: triggering one extra cycle just at the cap creates a new
    // socket.
    const before = MockWebSocket.instances.length;
    act(() => MockWebSocket.last()!.simulateClose());
    act(() => {
      vi.advanceTimersByTime(BACKOFF_MAX_MS);
    });
    expect(MockWebSocket.instances.length).toBe(before + 1);
  });

  it('cancels pending reconnect on unmount', () => {
    const { unmount } = renderHook(() =>
      useReconnectingWebSocket('wss://example.com/x')
    );
    act(() => MockWebSocket.last()!.simulateClose());
    const before = MockWebSocket.instances.length;
    unmount();
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(MockWebSocket.instances.length).toBe(before);
  });
});
