import { useEffect, useRef, useState } from 'react';
import { WsClient, type WsEventHandler } from '../lib/ws_client';

export type ConnectionState = 'connecting' | 'open' | 'closed' | 'error';

export function useWebSocket(url: string | null) {
  const [state, setState] = useState<ConnectionState>('closed');
  const clientRef = useRef<WsClient | null>(null);
  const handlersRef = useRef<Set<(e: WsEventHandler) => void>>(new Set());

  useEffect(() => {
    if (!url) return;
    const client = new WsClient(url);
    clientRef.current = client;
    setState('connecting');

    const off = client.on((e) => {
      if (e.type === 'open') setState('open');
      else if (e.type === 'close') setState('closed');
      else if (e.type === 'error') setState('error');
      for (const h of handlersRef.current) h(e);
    });
    client.connect();

    return () => {
      off();
      client.close();
      clientRef.current = null;
    };
  }, [url]);

  return {
    state,
    client: clientRef.current,
    on: (h: (e: WsEventHandler) => void) => {
      handlersRef.current.add(h);
      return () => handlersRef.current.delete(h);
    },
  };
}
