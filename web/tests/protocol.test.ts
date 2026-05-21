import { describe, it, expect } from 'vitest';
import { parseControlMessage, encodeControlMessage, type ControlMessage } from '../src/lib/protocol';

describe('protocol', () => {
  it('encodes and decodes phone_hello', () => {
    const msg: ControlMessage = {
      type: 'phone_hello',
      token: 'a'.repeat(43),
      cols: 80,
      rows: 24,
    };
    const s = encodeControlMessage(msg);
    const back = parseControlMessage(s);
    expect(back.type).toBe('phone_hello');
    if (back.type === 'phone_hello') {
      expect(back.cols).toBe(80);
    }
  });

  it('rejects non-object', () => {
    expect(() => parseControlMessage('"foo"')).toThrow();
  });

  it('parses error message', () => {
    const msg: ControlMessage = {
      type: 'error',
      code: 'invalid_token',
      message: 'nope',
    };
    const s = encodeControlMessage(msg);
    const back = parseControlMessage(s);
    expect(back.type).toBe('error');
  });
});
