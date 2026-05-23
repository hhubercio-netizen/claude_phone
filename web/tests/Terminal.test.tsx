import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';

// xterm needs canvas + ResizeObserver. We mock both so the wrapper logic can be
// exercised in jsdom without dragging in headless-browser tooling.
class FakeXTerm {
  static lastInstance: FakeXTerm | null = null;
  options: Record<string, unknown>;
  written: (string | Uint8Array)[] = [];
  onDataCb: ((d: string) => void) | null = null;
  onResizeCb: ((s: { cols: number; rows: number }) => void) | null = null;
  onScrollCb: (() => void) | null = null;
  disposed = false;
  focused = false;
  opened = false;
  scrollToBottomCalls = 0;
  // The buffer state used by useTerminal.computeAtBottom — by default the
  // viewport is anchored at the latest output (viewportY >= baseY).
  buffer = {
    active: { viewportY: 0, baseY: 0, length: 0 },
  };

  constructor(options: Record<string, unknown>) {
    this.options = options;
    FakeXTerm.lastInstance = this;
  }
  loadAddon(_addon: unknown): void {}
  open(_container: HTMLElement): void {
    this.opened = true;
  }
  onData(cb: (d: string) => void): void {
    this.onDataCb = cb;
  }
  onResize(cb: (s: { cols: number; rows: number }) => void): void {
    this.onResizeCb = cb;
  }
  onScroll(cb: () => void): void {
    this.onScrollCb = cb;
  }
  write(data: string | Uint8Array): void {
    this.written.push(data);
  }
  focus(): void {
    this.focused = true;
  }
  dispose(): void {
    this.disposed = true;
  }
  scrollToBottom(): void {
    this.scrollToBottomCalls += 1;
    this.buffer.active.viewportY = this.buffer.active.baseY;
  }
}

class FakeFitAddon {
  static instances: FakeFitAddon[] = [];
  fitCalls = 0;
  constructor() {
    FakeFitAddon.instances.push(this);
  }
  fit(): void {
    this.fitCalls += 1;
  }
}

vi.mock('@xterm/xterm', () => ({ Terminal: FakeXTerm }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: FakeFitAddon }));

// jsdom has no ResizeObserver — install a minimal capture so we can fire it.
class MockResizeObserver {
  static instances: MockResizeObserver[] = [];
  cb: ResizeObserverCallback;
  observed: Element[] = [];
  disconnected = false;
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb;
    MockResizeObserver.instances.push(this);
  }
  observe(el: Element): void {
    this.observed.push(el);
  }
  disconnect(): void {
    this.disconnected = true;
  }
  unobserve(): void {}
  fire(): void {
    this.cb([], this as unknown as ResizeObserver);
  }
}

beforeEach(() => {
  (globalThis as unknown as { ResizeObserver: typeof MockResizeObserver }).ResizeObserver =
    MockResizeObserver;
  MockResizeObserver.instances = [];
  FakeFitAddon.instances = [];
  FakeXTerm.lastInstance = null;
});

const { Terminal } = await import('../src/components/Terminal/Terminal');

describe('Terminal', () => {
  it('mounts an xterm instance into its container', () => {
    render(
      <Terminal onInputBytes={() => {}} onResize={() => {}} />,
    );
    expect(FakeXTerm.lastInstance).not.toBeNull();
    expect(FakeXTerm.lastInstance!.opened).toBe(true);
  });

  it('forwards keystrokes from xterm onData as utf-8 bytes', () => {
    const onInput = vi.fn();
    render(<Terminal onInputBytes={onInput} onResize={() => {}} />);
    FakeXTerm.lastInstance!.onDataCb!('ab');
    expect(onInput).toHaveBeenCalledTimes(1);
    const got = onInput.mock.calls[0][0] as Uint8Array;
    expect(Array.from(got)).toEqual([0x61, 0x62]);
  });

  it('forwards xterm onResize to onResize prop', () => {
    const onResize = vi.fn();
    render(<Terminal onInputBytes={() => {}} onResize={onResize} />);
    FakeXTerm.lastInstance!.onResizeCb!({ cols: 132, rows: 50 });
    expect(onResize).toHaveBeenCalledWith(132, 50);
  });

  it('hands a writeHandle to the parent that pipes bytes into xterm.write', () => {
    let writer: ((b: Uint8Array) => void) | null = null;
    render(
      <Terminal
        onInputBytes={() => {}}
        onResize={() => {}}
        writeHandle={(w) => {
          writer = w;
        }}
      />,
    );
    expect(writer).not.toBeNull();
    writer!(new Uint8Array([1, 2, 3]));
    expect(FakeXTerm.lastInstance!.written.length).toBe(1);
    expect(FakeXTerm.lastInstance!.written[0]).toEqual(new Uint8Array([1, 2, 3]));
  });

  it('focuses xterm when the container is clicked', () => {
    const { container } = render(
      <Terminal onInputBytes={() => {}} onResize={() => {}} />,
    );
    fireEvent.click(container.firstChild as Element);
    expect(FakeXTerm.lastInstance!.focused).toBe(true);
  });

  it('disposes xterm and disconnects ResizeObserver on unmount', () => {
    const { unmount } = render(
      <Terminal onInputBytes={() => {}} onResize={() => {}} />,
    );
    const term = FakeXTerm.lastInstance!;
    const ro = MockResizeObserver.instances[0];
    unmount();
    expect(term.disposed).toBe(true);
    expect(ro.disconnected).toBe(true);
  });

  it('runs fit() at least once on mount (initial size)', () => {
    render(<Terminal onInputBytes={() => {}} onResize={() => {}} />);
    expect(FakeFitAddon.instances[0].fitCalls).toBeGreaterThanOrEqual(1);
  });

  it('re-fits when ResizeObserver fires', () => {
    render(<Terminal onInputBytes={() => {}} onResize={() => {}} />);
    const initialFits = FakeFitAddon.instances[0].fitCalls;
    MockResizeObserver.instances[0].fire();
    expect(FakeFitAddon.instances[0].fitCalls).toBeGreaterThan(initialFits);
  });
});
