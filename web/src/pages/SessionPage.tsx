import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import { ActionBar } from '../components/ActionBar/ActionBar';
import { MobileLayout } from '../components/Layout/MobileLayout';
import { PasteModal } from '../components/PasteModal/PasteModal';
import { Terminal } from '../components/Terminal/Terminal';
import type { TerminalHandle } from '../components/Terminal/Terminal';
import { useFontSize } from '../hooks/useFontSize';
import { useReconnectingWebSocket } from '../hooks/useReconnect';
import { useWakeLock } from '../hooks/useWakeLock';
import { useSessionStore } from '../store/session';
import type { ControlMessage } from '../lib/protocol';

// Defense-in-depth: must match server-side SessionToken::parse() (43 chars,
// base64url charset). Anything else gets rejected client-side before we even
// attempt a WebSocket — avoids surfacing a malformed path to the gateway and
// avoids any chance of URL injection if React Router ever forwarded raw input.
const TOKEN_RE = /^[A-Za-z0-9_-]{43}$/;

function isValidToken(t: string | undefined): t is string {
  return typeof t === 'string' && TOKEN_RE.test(t);
}

function gatewayWsUrl(token: string): string {
  // Always derive from current origin so it works behind Cloudflare with TLS.
  // encodeURIComponent is redundant after isValidToken (base64url has no URI-
  // unsafe chars), but kept as belt-and-braces in case the validator changes.
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/api/phone/${encodeURIComponent(token)}`;
}

export function SessionPage() {
  const { token } = useParams<{ token: string }>();
  const setPeer = useSessionStore((s) => s.setPeerConnected);
  const setServerSessionId = useSessionStore((s) => s.setServerSessionId);

  const writeRef = useRef<((bytes: Uint8Array) => void) | null>(null);
  const termHandleRef = useRef<TerminalHandle | null>(null);

  const tokenValid = isValidToken(token);
  const url = tokenValid ? gatewayWsUrl(token) : null;
  const { state, client, on } = useReconnectingWebSocket(url);

  const font = useFontSize();
  const [pasteOpen, setPasteOpen] = useState(false);

  // Hold the screen on while a valid session page is mounted. The hook is a
  // no-op on browsers without the Wake Lock API.
  useWakeLock(tokenValid);

  // Send phone_hello after EVERY open — the reconnecting hook re-opens the WS
  // on backoff after network blips, and the gateway expects phone_hello to be
  // the first message on each fresh socket. The gateway-side sticky session
  // matches us back to the same Session by token and replays buffered output.
  useEffect(() => {
    if (!client || !tokenValid) return;
    const off = on((e) => {
      if (e.type === 'open') {
        client.sendControl({
          type: 'phone_hello',
          token: token!,
          cols: 80,
          rows: 24,
          user_agent: navigator.userAgent,
        });
      } else if (e.type === 'control') {
        handleControl(e.message);
      } else if (e.type === 'binary') {
        writeRef.current?.(new Uint8Array(e.data));
      }
    });
    return off;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, on, token, tokenValid]);

  if (!tokenValid) {
    return <div className="p-4 text-claude-err">Bad token format.</div>;
  }

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

  function handlePasteSend(bytes: Uint8Array) {
    // Route through the same WS as keystrokes. Gateway treats it as plain
    // binary input to the wrapper PTY — identical to a fast typist.
    handleInput(bytes);
  }

  return (
    <>
      <MobileLayout
        header={
          <SessionHeader
            wsState={state}
            fontSize={font.size}
            onFontInc={font.inc}
            onFontDec={font.dec}
            onPaste={() => setPasteOpen(true)}
          />
        }
        body={
          <Terminal
            onInputBytes={handleInput}
            onResize={handleResize}
            writeHandle={(w) => (writeRef.current = w)}
            controlHandle={(h) => (termHandleRef.current = h)}
            fontSize={font.size}
          />
        }
        footer={<ActionBar onKey={handleInput} />}
      />
      <PasteModal
        open={pasteOpen}
        onClose={() => setPasteOpen(false)}
        onSend={handlePasteSend}
      />
    </>
  );
}

interface HeaderProps {
  wsState: string;
  fontSize: number;
  onFontInc: () => void;
  onFontDec: () => void;
  onPaste: () => void;
}

function SessionHeader({ wsState, fontSize, onFontInc, onFontDec, onPaste }: HeaderProps) {
  const peer = useSessionStore((s) => s.peerConnected);
  return (
    <header className="px-3 py-1 border-b border-claude-panelBorder text-xs flex justify-between items-center gap-2">
      <div className="flex items-center gap-3 min-w-0">
        <span>
          WS:{' '}
          <span className={wsState === 'open' ? 'text-claude-ok' : 'text-claude-err'}>
            {wsState}
          </span>
        </span>
        <span className="truncate">
          Wrapper:{' '}
          {peer ? (
            <span className="text-claude-ok">paired</span>
          ) : (
            <span className="text-claude-muted">waiting</span>
          )}
        </span>
      </div>
      <div className="flex items-center gap-1">
        {/* ASCII hyphen instead of U+2212 (−) — the unicode minus is missing
            from several mobile monospace fallbacks and renders as a tofu
            box that reads as a stray "L" or "*" character. */}
        <HeaderBtn label="A-" onClick={onFontDec} ariaLabel="Decrease font size" />
        <span className="text-claude-muted tabular-nums w-6 text-center">{fontSize}</span>
        <HeaderBtn label="A+" onClick={onFontInc} ariaLabel="Increase font size" />
        <HeaderBtn label="Paste" onClick={onPaste} ariaLabel="Open paste dialog" />
      </div>
    </header>
  );
}

function HeaderBtn({
  label,
  onClick,
  ariaLabel,
}: {
  label: string;
  onClick: () => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={ariaLabel}
      className="px-2 py-0.5 rounded border border-claude-panelBorder bg-claude-panelBg text-claude-fg active:bg-white active:text-black text-xs"
    >
      {label}
    </button>
  );
}
