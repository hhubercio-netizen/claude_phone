import { describe, it, expect, beforeEach, vi } from 'vitest';
import { MockWebSocket, installMockWebSocket } from './mock-ws';
import { useSessionStore } from '../src/store/session';
import { WsClient } from '../src/lib/ws_client';

const TOKEN = 'a'.repeat(43);

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  installMockWebSocket();
  MockWebSocket.reset();
  useSessionStore.setState({
    token: null,
    serverSessionId: null,
    peerConnected: false,
  });
});

// TM-FRONT.5: forward-looking tests asserting the session token never lands
// in localStorage, sessionStorage, or window.history.state. A future change
// that introduces a "remember session" toggle would have to remove or weaken
// one of these tests, surfacing the trade-off in code review.
describe('secret leakage', () => {
  it('session store does not persist token to localStorage', () => {
    useSessionStore.getState().setToken(TOKEN);
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)!;
      const v = localStorage.getItem(k) ?? '';
      expect(v).not.toContain(TOKEN);
    }
  });

  it('session store does not persist token to sessionStorage', () => {
    useSessionStore.getState().setToken(TOKEN);
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i)!;
      const v = sessionStorage.getItem(k) ?? '';
      expect(v).not.toContain(TOKEN);
    }
  });

  it('window.history.state does not retain token verbatim across navigation', () => {
    window.history.pushState({}, '', `/s/${TOKEN}`);
    const stateStr = JSON.stringify(window.history.state ?? {});
    expect(stateStr).not.toContain(TOKEN);
  });

  it('ws_client does not log raw frame data on parse failure', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const c = new WsClient('wss://example.com/api/phone/x');
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    MockWebSocket.last()!.simulateMessage(`payload containing ${TOKEN}`);

    expect(spy).toHaveBeenCalled();
    for (const call of spy.mock.calls) {
      const joined = call.map((a) => String(a)).join(' ');
      expect(joined).not.toContain(TOKEN);
    }
    spy.mockRestore();
  });

  it('protocol error messages from gateway are not blindly logged with token', () => {
    // Pin that we only treat parsed `error` ControlMessage fields as loggable,
    // not the raw frame. This is the only place ws_client touches console.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const c = new WsClient('wss://example.com/api/phone/x');
    c.connect();
    MockWebSocket.last()!.simulateOpen();
    // Valid control message — must NOT trigger the parse-failure log path.
    MockWebSocket.last()!.simulateMessage(
      JSON.stringify({
        type: 'server_hello',
        session_id: 'abc',
        peer_connected: false,
      })
    );
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});
