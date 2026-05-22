import { describe, it, expect, beforeEach } from 'vitest';
import { useSessionStore } from '../src/store/session';

beforeEach(() => {
  useSessionStore.setState({
    token: null,
    serverSessionId: null,
    peerConnected: false,
  });
});

describe('useSessionStore', () => {
  it('starts unpaired', () => {
    const s = useSessionStore.getState();
    expect(s.token).toBeNull();
    expect(s.serverSessionId).toBeNull();
    expect(s.peerConnected).toBe(false);
  });

  it('setToken updates token', () => {
    useSessionStore.getState().setToken('abc');
    expect(useSessionStore.getState().token).toBe('abc');
  });

  it('setServerSessionId updates server session id', () => {
    useSessionStore.getState().setServerSessionId('srv-1');
    expect(useSessionStore.getState().serverSessionId).toBe('srv-1');
  });

  it('setPeerConnected toggles', () => {
    useSessionStore.getState().setPeerConnected(true);
    expect(useSessionStore.getState().peerConnected).toBe(true);
    useSessionStore.getState().setPeerConnected(false);
    expect(useSessionStore.getState().peerConnected).toBe(false);
  });

  it('reset returns to initial state', () => {
    const s = useSessionStore.getState();
    s.setToken('x');
    s.setServerSessionId('y');
    s.setPeerConnected(true);
    s.reset();
    const after = useSessionStore.getState();
    expect(after.token).toBeNull();
    expect(after.serverSessionId).toBeNull();
    expect(after.peerConnected).toBe(false);
  });
});
