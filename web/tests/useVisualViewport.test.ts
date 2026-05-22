import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useVisualViewport } from '../src/hooks/useVisualViewport';

type Listener = (e: any) => void;

class FakeViewport {
  height: number;
  width: number;
  private listeners: Record<string, Listener[]> = { resize: [], scroll: [] };
  constructor(height: number, width: number) {
    this.height = height;
    this.width = width;
  }
  addEventListener(type: string, fn: Listener) {
    (this.listeners[type] ||= []).push(fn);
  }
  removeEventListener(type: string, fn: Listener) {
    this.listeners[type] = (this.listeners[type] || []).filter((l) => l !== fn);
  }
  fire(type: string) {
    for (const l of this.listeners[type] || []) l({});
  }
  listenerCount(type: string): number {
    return (this.listeners[type] || []).length;
  }
}

let fake: FakeViewport;
let originalViewport: any;

beforeEach(() => {
  fake = new FakeViewport(800, 400);
  originalViewport = (window as any).visualViewport;
  (window as any).visualViewport = fake;
});

afterEach(() => {
  (window as any).visualViewport = originalViewport;
});

describe('useVisualViewport', () => {
  it('returns the initial viewport size', () => {
    const { result } = renderHook(() => useVisualViewport());
    expect(result.current.height).toBe(800);
    expect(result.current.width).toBe(400);
    expect(result.current.keyboardOpen).toBe(false);
  });

  it('updates on resize', () => {
    const { result } = renderHook(() => useVisualViewport());
    act(() => {
      fake.height = 500;
      fake.fire('resize');
    });
    expect(result.current.height).toBe(500);
  });

  it('sets keyboardOpen when inner height exceeds viewport by 100+', () => {
    (window as any).innerHeight = 800;
    fake.height = 600;
    const { result } = renderHook(() => useVisualViewport());
    act(() => {
      fake.fire('resize');
    });
    expect(result.current.keyboardOpen).toBe(true);
  });

  it('removes listeners on unmount', () => {
    const { unmount } = renderHook(() => useVisualViewport());
    expect(fake.listenerCount('resize')).toBe(1);
    expect(fake.listenerCount('scroll')).toBe(1);
    unmount();
    expect(fake.listenerCount('resize')).toBe(0);
    expect(fake.listenerCount('scroll')).toBe(0);
  });
});
