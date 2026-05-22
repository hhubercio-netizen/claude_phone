import { describe, it, expect, beforeEach, vi } from 'vitest';
import { MockWebSocket, installMockWebSocket } from './mock-ws';
import { WsClient, type WsEventHandler } from '../src/lib/ws_client';
import { encodeControlMessage } from '../src/lib/protocol';

beforeEach(() => {
  installMockWebSocket();
  MockWebSocket.reset();
});

function collect(client: WsClient): WsEventHandler[] {
  const events: WsEventHandler[] = [];
  client.on((e) => events.push(e));
  return events;
}

describe('WsClient', () => {
  it('connects once and emits open', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    const events = collect(c);
    c.connect();
    expect(MockWebSocket.instances.length).toBe(1);
    MockWebSocket.last()!.simulateOpen();
    expect(events.map((e) => e.type)).toEqual(['open']);
  });

  it('does not double-connect when connect() is called twice', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    c.connect();
    c.connect();
    expect(MockWebSocket.instances.length).toBe(1);
  });

  it('parses incoming control messages and emits control event', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    const events = collect(c);
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    const payload = encodeControlMessage({
      type: 'server_hello',
      session_id: 'sess-1',
      peer_connected: false,
    });
    MockWebSocket.last()!.simulateMessage(payload);

    const ctrl = events.find((e) => e.type === 'control');
    expect(ctrl).toBeTruthy();
    if (ctrl && ctrl.type === 'control') {
      expect(ctrl.message.type).toBe('server_hello');
    }
  });

  it('emits binary event for ArrayBuffer messages', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    const events = collect(c);
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    const buf = new Uint8Array([1, 2, 3]).buffer;
    MockWebSocket.last()!.simulateMessage(buf);

    const bin = events.find((e) => e.type === 'binary');
    expect(bin).toBeTruthy();
    if (bin && bin.type === 'binary') {
      expect(new Uint8Array(bin.data)).toEqual(new Uint8Array([1, 2, 3]));
    }
  });

  it('forwards sendControl to the underlying socket', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    c.sendControl({ type: 'resize', cols: 100, rows: 30 });
    expect(MockWebSocket.last()!.sent.length).toBe(1);
    expect(MockWebSocket.last()!.sent[0]).toContain('"resize"');
  });

  it('forwards sendBinary to the underlying socket', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    const bytes = new Uint8Array([9, 8, 7]);
    c.sendBinary(bytes);
    expect(MockWebSocket.last()!.sent.length).toBe(1);
    expect(MockWebSocket.last()!.sent[0]).toBe(bytes);
  });

  it('emits close event with code and reason', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    const events = collect(c);
    c.connect();
    MockWebSocket.last()!.simulateClose(1006, 'gone');
    const close = events.find((e) => e.type === 'close');
    expect(close).toBeTruthy();
    if (close && close.type === 'close') {
      expect(close.code).toBe(1006);
      expect(close.reason).toBe('gone');
    }
  });

  it('logs sanitized error on bad control message — never raw frame data', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const c = new WsClient('wss://example.com/api/phone/abc');
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    const secret = 'TOKEN_THAT_SHOULD_NEVER_APPEAR_IN_LOGS';
    MockWebSocket.last()!.simulateMessage(`not json with ${secret} inside`);

    expect(spy).toHaveBeenCalled();
    const call = spy.mock.calls[0];
    expect(call[2]).toBe('<raw frame omitted>');
    const joined = call.map((a) => String(a)).join(' ');
    expect(joined).not.toContain(secret);
    spy.mockRestore();
  });

  it('removes listener returned by on()', () => {
    const c = new WsClient('wss://example.com/api/phone/abc');
    const seen: WsEventHandler[] = [];
    const off = c.on((e) => seen.push(e));
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    expect(seen.length).toBe(1);
    off();
    MockWebSocket.last()!.simulateClose();
    expect(seen.length).toBe(1);
  });
});
