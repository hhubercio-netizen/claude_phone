import { create } from 'zustand';

interface SessionState {
  token: string | null;
  serverSessionId: string | null;
  peerConnected: boolean;
  setToken: (t: string) => void;
  setServerSessionId: (id: string) => void;
  setPeerConnected: (b: boolean) => void;
  reset: () => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  token: null,
  serverSessionId: null,
  peerConnected: false,
  setToken: (token) => set({ token }),
  setServerSessionId: (id) => set({ serverSessionId: id }),
  setPeerConnected: (b) => set({ peerConnected: b }),
  reset: () =>
    set({ token: null, serverSessionId: null, peerConnected: false }),
}));
