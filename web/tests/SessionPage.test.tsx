import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { MockWebSocket, installMockWebSocket } from './mock-ws';
import { useSessionStore } from '../src/store/session';
import { encodeControlMessage } from '../src/lib/protocol';

// Stub the Terminal component so SessionPage tests don't try to mount xterm in
// jsdom. We capture the writeHandle so we can assert binary frames are
// forwarded into it.
const lastWriteHandleRef: { current: ((b: Uint8Array) => void) | null } = {
  current: null,
};
const inputHandlerRef: {
  current: ((b: Uint8Array) => void) | null;
  resize: ((c: number, r: number) => void) | null;
} = { current: null, resize: null };

vi.mock('../src/components/Terminal/Terminal', () => ({
  Terminal: (props: {
    onInputBytes: (b: Uint8Array) => void;
    onResize: (c: number, r: number) => void;
    writeHandle?: (w: (b: Uint8Array) => void) => void;
  }) => {
    inputHandlerRef.current = props.onInputBytes;
    inputHandlerRef.resize = props.onResize;
    if (props.writeHandle) {
      props.writeHandle((bytes) => {
        lastWriteHandleRef.current?.(bytes);
        // also remember the last-written bytes via the same ref slot
        (lastWriteHandleRef as { lastBytes?: Uint8Array }).lastBytes = bytes;
      });
    }
    return <div data-testid="terminal-stub" />;
  },
}));

// Import AFTER vi.mock so the mock is applied.
const { SessionPage } = await import('../src/pages/SessionPage');

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/s/:token" element={<SessionPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

const VALID_TOKEN = 'A'.repeat(43); // 43 base64url chars

beforeEach(() => {
  installMockWebSocket();
  MockWebSocket.reset();
  useSessionStore.getState().reset();
  lastWriteHandleRef.current = null;
  inputHandlerRef.current = null;
  inputHandlerRef.resize = null;
});

describe('SessionPage', () => {
  it('rejects a token shorter than 43 chars without opening a WebSocket', () => {
    renderAt('/s/short');
    expect(screen.getByText(/bad token format/i)).toBeInTheDocument();
    expect(MockWebSocket.instances.length).toBe(0);
  });

  it('rejects a token with invalid characters', () => {
    const bad = 'A'.repeat(42) + '!'; // 43 chars but `!` is not base64url
    renderAt(`/s/${encodeURIComponent(bad)}`);
    expect(screen.getByText(/bad token format/i)).toBeInTheDocument();
    expect(MockWebSocket.instances.length).toBe(0);
  });

  it('rejects a token longer than 43 chars', () => {
    const bad = 'A'.repeat(44);
    renderAt(`/s/${bad}`);
    expect(screen.getByText(/bad token format/i)).toBeInTheDocument();
    expect(MockWebSocket.instances.length).toBe(0);
  });

  it('opens a WebSocket for a valid token', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    expect(MockWebSocket.instances.length).toBe(1);
    // jsdom default origin is http://localhost (verify in next test)
  });

  it('uses ws:// when origin is http and wss:// when origin is https', () => {
    // jsdom's default location is http://localhost — so first check ws://
    renderAt(`/s/${VALID_TOKEN}`);
    expect(MockWebSocket.last()!.url).toMatch(/^ws:\/\//);
    expect(MockWebSocket.last()!.url).toContain(`/api/phone/${VALID_TOKEN}`);
  });

  it('sends phone_hello with the token after WS open', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    const sent = MockWebSocket.last()!.sent;
    expect(sent.length).toBe(1);
    const payload = JSON.parse(sent[0] as string);
    expect(payload.type).toBe('phone_hello');
    expect(payload.token).toBe(VALID_TOKEN);
    expect(payload.cols).toBe(80);
    expect(payload.rows).toBe(24);
  });

  it('sends phone_hello only once even if the open event fires multiple times', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    // helloSent gates re-send
    expect(MockWebSocket.last()!.sent.length).toBe(1);
  });

  it('stores server_session_id and peer_connected on server_hello', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
      MockWebSocket.last()!.simulateMessage(
        encodeControlMessage({
          type: 'server_hello',
          session_id: 'sess-abc',
          peer_connected: true,
        }),
      );
    });
    expect(useSessionStore.getState().serverSessionId).toBe('sess-abc');
    expect(useSessionStore.getState().peerConnected).toBe(true);
  });

  it('updates peer_connected on peer_status frame', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
      MockWebSocket.last()!.simulateMessage(
        encodeControlMessage({ type: 'peer_status', connected: false }),
      );
    });
    expect(useSessionStore.getState().peerConnected).toBe(false);
  });

  it('writes binary frames into the terminal write handle', () => {
    const received: Uint8Array[] = [];
    lastWriteHandleRef.current = (b) => received.push(b);
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
      MockWebSocket.last()!.simulateMessage(
        new Uint8Array([0x68, 0x69]).buffer, // "hi"
      );
    });
    expect(received.length).toBe(1);
    expect(Array.from(received[0])).toEqual([0x68, 0x69]);
  });

  it('forwards terminal input bytes via sendBinary', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    // simulate terminal emitting input
    act(() => {
      inputHandlerRef.current!(new Uint8Array([0x61, 0x62])); // "ab"
    });
    // First sent message is the JSON phone_hello; second is the binary input.
    const sent = MockWebSocket.last()!.sent;
    expect(sent.length).toBe(2);
    expect(sent[1]).toBeInstanceOf(Uint8Array);
    expect(Array.from(sent[1] as Uint8Array)).toEqual([0x61, 0x62]);
  });

  it('forwards terminal resize as a control message', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    act(() => {
      inputHandlerRef.resize!(120, 40);
    });
    const sent = MockWebSocket.last()!.sent;
    // Last one should be the resize JSON.
    const resize = JSON.parse(sent[sent.length - 1] as string);
    expect(resize.type).toBe('resize');
    expect(resize.cols).toBe(120);
    expect(resize.rows).toBe(40);
  });

  it('logs gateway errors but never the raw token', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
      MockWebSocket.last()!.simulateMessage(
        encodeControlMessage({
          type: 'error',
          code: 'invalid_token',
          message: 'no such session',
        }),
      );
    });
    expect(spy).toHaveBeenCalled();
    const joined = spy.mock.calls
      .flatMap((args) => args.map((a) => String(a)))
      .join(' ');
    expect(joined).not.toContain(VALID_TOKEN);
    spy.mockRestore();
  });

  it('does not leak the token via document.title, localStorage, or sessionStorage', () => {
    renderAt(`/s/${VALID_TOKEN}`);
    act(() => {
      MockWebSocket.last()!.simulateOpen();
    });
    expect(document.title).not.toContain(VALID_TOKEN);
    // The store must not persist the token (only serverSessionId + peerConnected).
    expect(useSessionStore.getState().token).toBeNull();
    expect(Object.keys(localStorage)).toHaveLength(0);
    expect(Object.keys(sessionStorage)).toHaveLength(0);
  });
});
