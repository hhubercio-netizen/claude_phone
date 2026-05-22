import { type ControlMessage, encodeControlMessage, parseControlMessage } from './protocol';

export type WsEventHandler =
  | { type: 'open' }
  | { type: 'close'; code: number; reason: string }
  | { type: 'error' }
  | { type: 'control'; message: ControlMessage }
  | { type: 'binary'; data: ArrayBuffer };

export class WsClient {
  private ws: WebSocket | null = null;
  private listeners = new Set<(e: WsEventHandler) => void>();

  constructor(private url: string) {}

  connect() {
    if (this.ws) return;
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => this.emit({ type: 'open' });
    ws.onclose = (e) => {
      this.ws = null;
      this.emit({ type: 'close', code: e.code, reason: e.reason });
    };
    ws.onerror = () => this.emit({ type: 'error' });
    ws.onmessage = (e) => {
      if (typeof e.data === 'string') {
        try {
          this.emit({ type: 'control', message: parseControlMessage(e.data) });
        } catch (err) {
          console.error('bad control message', err, e.data);
        }
      } else {
        this.emit({ type: 'binary', data: e.data as ArrayBuffer });
      }
    };
    this.ws = ws;
  }

  sendControl(msg: ControlMessage) {
    this.ws?.send(encodeControlMessage(msg));
  }

  sendBinary(bytes: ArrayBuffer | Uint8Array) {
    this.ws?.send(bytes);
  }

  close() {
    this.ws?.close();
    this.ws = null;
  }

  on(handler: (e: WsEventHandler) => void): () => void {
    this.listeners.add(handler);
    return () => this.listeners.delete(handler);
  }

  private emit(e: WsEventHandler) {
    for (const l of this.listeners) {
      l(e);
    }
  }
}
