type Listener = (event: any) => void;

export class MockWebSocket {
  static instances: MockWebSocket[] = [];

  readonly url: string;
  readyState: number = 0; // CONNECTING
  binaryType: string = 'blob';
  sent: (string | ArrayBufferLike | Blob | ArrayBufferView)[] = [];

  onopen: ((e: any) => void) | null = null;
  onclose: ((e: any) => void) | null = null;
  onerror: ((e: any) => void) | null = null;
  onmessage: ((e: any) => void) | null = null;

  private listeners: Record<string, Listener[]> = {
    open: [],
    close: [],
    error: [],
    message: [],
  };

  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  addEventListener(type: string, listener: Listener): void {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(listener);
  }

  removeEventListener(type: string, listener: Listener): void {
    this.listeners[type] = (this.listeners[type] ?? []).filter((l) => l !== listener);
  }

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = MockWebSocket.CLOSED;
    this.dispatch('close', { code: 1000, reason: 'mock close' });
  }

  // Test helpers
  simulateOpen(): void {
    this.readyState = MockWebSocket.OPEN;
    this.dispatch('open', {});
  }

  simulateMessage(data: string | ArrayBuffer): void {
    this.dispatch('message', { data });
  }

  simulateError(): void {
    this.dispatch('error', { error: new Error('mock error') });
  }

  simulateClose(code = 1006, reason = 'abnormal'): void {
    this.readyState = MockWebSocket.CLOSED;
    this.dispatch('close', { code, reason });
  }

  private dispatch(type: string, event: any): void {
    for (const l of this.listeners[type] ?? []) l(event);
    // The WsClient under test uses on-* properties (ws.onopen, ws.onmessage, etc.)
    const onProp = (this as any)['on' + type];
    if (typeof onProp === 'function') onProp(event);
  }

  static reset(): void {
    MockWebSocket.instances = [];
  }

  static last(): MockWebSocket | undefined {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

export function installMockWebSocket(): typeof MockWebSocket {
  (globalThis as any).WebSocket = MockWebSocket;
  return MockWebSocket;
}
