import { describe, it, expect, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { MockWebSocket, installMockWebSocket } from './mock-ws';
import { useWebSocket } from '../src/hooks/useWebSocket';

beforeEach(() => {
  installMockWebSocket();
  MockWebSocket.reset();
});

describe('useWebSocket', () => {
  it('does not connect when url is null', () => {
    renderHook(() => useWebSocket(null));
    expect(MockWebSocket.instances.length).toBe(0);
  });

  it('connects and transitions to open on socket open', () => {
    const { result } = renderHook(() => useWebSocket('wss://example.com/x'));
    expect(MockWebSocket.instances.length).toBe(1);
    expect(result.current.state).toBe('connecting');
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    expect(result.current.state).toBe('open');
  });

  it('transitions to closed on socket close', () => {
    const { result } = renderHook(() => useWebSocket('wss://example.com/x'));
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    act(() => {
      MockWebSocket.last()!.simulateClose();
    });
    expect(result.current.state).toBe('closed');
  });

  it('closes the socket on unmount', () => {
    const { unmount } = renderHook(() => useWebSocket('wss://example.com/x'));
    const ws = MockWebSocket.last()!;
    unmount();
    expect(ws.readyState).toBe(MockWebSocket.CLOSED);
  });

  it('fans events out to handlers registered via on()', () => {
    const { result } = renderHook(() => useWebSocket('wss://example.com/x'));
    const seen: string[] = [];
    act(() => {
      result.current.on((e) => seen.push(e.type));
    });
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    expect(seen).toContain('open');
  });
});
