import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import { ActionBar } from '../components/ActionBar/ActionBar';
import { MobileLayout } from '../components/Layout/MobileLayout';
import { Terminal } from '../components/Terminal/Terminal';
import { useWebSocket } from '../hooks/useWebSocket';
import { useSessionStore } from '../store/session';
import type { ControlMessage } from '../lib/protocol';

function gatewayWsUrl(token: string): string {
  // Always derive from current origin so it works behind Cloudflare with TLS
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/api/phone/${token}`;
}

export function SessionPage() {
  const { token } = useParams<{ token: string }>();
  const setPeer = useSessionStore((s) => s.setPeerConnected);
  const setServerSessionId = useSessionStore((s) => s.setServerSessionId);

  const writeRef = useRef<((bytes: Uint8Array) => void) | null>(null);
  const [helloSent, setHelloSent] = useState(false);

  if (!token || token.length !== 43) {
    return <div className="p-4 text-claude-err">Bad token format.</div>;
  }

  const url = gatewayWsUrl(token);
  const { state, client, on } = useWebSocket(url);

  // Send phone_hello after open
  useEffect(() => {
    if (!client) return;
    const off = on((e) => {
      if (e.type === 'open' && !helloSent) {
        client.sendControl({
          type: 'phone_hello',
          token: token!,
          cols: 80,
          rows: 24,
          user_agent: navigator.userAgent,
        });
        setHelloSent(true);
      } else if (e.type === 'control') {
        handleControl(e.message);
      } else if (e.type === 'binary') {
        writeRef.current?.(new Uint8Array(e.data));
      }
    });
    return off;
  }, [client, on, token, helloSent]);

  function handleControl(msg: ControlMessage) {
    if (msg.type === 'server_hello') {
      setServerSessionId(msg.session_id);
      setPeer(msg.peer_connected);
    } else if (msg.type === 'peer_status') {
      setPeer(msg.connected);
    } else if (msg.type === 'error') {
      console.error('gateway error', msg.code, msg.message);
    }
  }

  function handleInput(bytes: Uint8Array) {
    client?.sendBinary(bytes);
  }

  function handleResize(cols: number, rows: number) {
    client?.sendControl({ type: 'resize', cols, rows });
  }

  return (
    <MobileLayout
      header={<ConnectionStatus state={state} />}
      body={
        <Terminal
          onInputBytes={handleInput}
          onResize={handleResize}
          writeHandle={(w) => (writeRef.current = w)}
        />
      }
      footer={<ActionBar onKey={handleInput} />}
    />
  );
}

function ConnectionStatus({ state }: { state: string }) {
  const peer = useSessionStore((s) => s.peerConnected);
  return (
    <header className="px-3 py-1 border-b border-claude-panelBorder text-xs flex justify-between">
      <span>
        WS: <span className={state === 'open' ? 'text-claude-ok' : 'text-claude-err'}>{state}</span>
      </span>
      <span>
        Wrapper: {peer ? <span className="text-claude-ok">paired</span> : <span className="text-claude-muted">waiting</span>}
      </span>
    </header>
  );
}
