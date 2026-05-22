import { describe, it, expect } from 'vitest';
import {
  parseControlMessage,
  encodeControlMessage,
  type ControlMessage,
} from '../src/lib/protocol';

function roundtrip(msg: ControlMessage): ControlMessage {
  return parseControlMessage(encodeControlMessage(msg));
}

describe('protocol', () => {
  it('encodes and decodes wrapper_hello', () => {
    const msg: ControlMessage = {
      type: 'wrapper_hello',
      api_key: 'k'.repeat(43),
      token: 't'.repeat(43),
      cols: 80,
      rows: 24,
    };
    const back = roundtrip(msg);
    expect(back.type).toBe('wrapper_hello');
    if (back.type === 'wrapper_hello') {
      expect(back.cols).toBe(80);
      expect(back.token).toBe('t'.repeat(43));
    }
  });

  it('encodes and decodes phone_hello', () => {
    const msg: ControlMessage = {
      type: 'phone_hello',
      token: 'a'.repeat(43),
      cols: 80,
      rows: 24,
    };
    const back = roundtrip(msg);
    expect(back.type).toBe('phone_hello');
    if (back.type === 'phone_hello') {
      expect(back.cols).toBe(80);
    }
  });

  it('encodes and decodes server_hello', () => {
    const msg: ControlMessage = {
      type: 'server_hello',
      session_id: 's-1',
      peer_connected: true,
    };
    const back = roundtrip(msg);
    expect(back.type).toBe('server_hello');
    if (back.type === 'server_hello') {
      expect(back.peer_connected).toBe(true);
    }
  });

  it('encodes and decodes resize', () => {
    const msg: ControlMessage = { type: 'resize', cols: 100, rows: 40 };
    const back = roundtrip(msg);
    expect(back.type).toBe('resize');
    if (back.type === 'resize') {
      expect(back.cols).toBe(100);
      expect(back.rows).toBe(40);
    }
  });

  it('encodes and decodes peer_status', () => {
    const back = roundtrip({ type: 'peer_status', connected: true });
    expect(back.type).toBe('peer_status');
  });

  it('encodes and decodes close with optional reason', () => {
    const back = roundtrip({ type: 'close', reason: 'bye' });
    expect(back.type).toBe('close');
  });

  it('rejects non-object', () => {
    expect(() => parseControlMessage('"foo"')).toThrow();
  });

  it('rejects null', () => {
    expect(() => parseControlMessage('null')).toThrow();
  });

  it('rejects object without type field', () => {
    expect(() => parseControlMessage('{"foo":"bar"}')).toThrow();
  });

  it('rejects malformed JSON', () => {
    expect(() => parseControlMessage('{not json')).toThrow();
  });

  it('parses error message variant', () => {
    const msg: ControlMessage = {
      type: 'error',
      code: 'invalid_token',
      message: 'nope',
    };
    const back = roundtrip(msg);
    expect(back.type).toBe('error');
    if (back.type === 'error') {
      expect(back.code).toBe('invalid_token');
    }
  });
});
