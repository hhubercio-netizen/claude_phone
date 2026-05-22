import { useEffect, useRef, useState } from 'react';
import { WsClient, type WsEventHandler } from '../lib/ws_client';

export type ConnectionState =
  | 'connecting'
  | 'open'
  | 'closed'
  | 'error'
  | 'reconnecting';

const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 16000];

export interface ReconnectingWs {
  state: ConnectionState;
  client: WsClient | null;
  on: (h: (e: WsEventHandler) => void) => () => void;
}

export function useReconnectingWebSocket(url: string | null): ReconnectingWs {
  const [state, setState] = useState<ConnectionState>('closed');
  const clientRef = useRef<WsClient | null>(null);
  const handlersRef = useRef<Set<(e: WsEventHandler) => void>>(new Set());
  const attemptRef = useRef(0);
  const timerRef = useRef<number | null>(null);
  const aliveRef = useRef(false);

  useEffect(() => {
    if (!url) return;
    aliveRef.current = true;

    const connect = () => {
      if (!aliveRef.current) return;
      setState('connecting');
      const client = new WsClient(url);
      clientRef.current = client;

      client.on((e) => {
        if (e.type === 'open') {
          attemptRef.current = 0;
          setState('open');
        } else if (e.type === 'close') {
          if (aliveRef.current) {
            setState('reconnecting');
            scheduleReconnect();
          } else {
            setState('closed');
          }
        } else if (e.type === 'error') {
          setState('error');
        }
        for (const h of handlersRef.current) h(e);
      });
      client.connect();
    };

    const scheduleReconnect = () => {
      const i = Math.min(attemptRef.current, BACKOFF_MS.length - 1);
      const delay = BACKOFF_MS[i] + Math.random() * 250;
      attemptRef.current += 1;
      timerRef.current = window.setTimeout(connect, delay);
    };

    connect();

    return () => {
      aliveRef.current = false;
      if (timerRef.current) window.clearTimeout(timerRef.current);
      clientRef.current?.close();
      clientRef.current = null;
    };
  }, [url]);

  return {
    state,
    client: clientRef.current,
    on: (h) => {
      handlersRef.current.add(h);
      return () => {
        handlersRef.current.delete(h);
      };
    },
  };
}
