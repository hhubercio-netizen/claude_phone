import { describe, it, expect } from 'vitest';
import { DEFAULT_KEYS } from '../src/components/ActionBar/keys';

function findBytes(label: string): number[] {
  const k = DEFAULT_KEYS.find((k) => k.label === label);
  if (!k) throw new Error(`no key with label ${label}`);
  return k.bytes;
}

describe('ActionBar DEFAULT_KEYS', () => {
  it('Esc emits 0x1b', () => {
    expect(findBytes('Esc')).toEqual([0x1b]);
  });

  it('Tab emits 0x09', () => {
    expect(findBytes('Tab')).toEqual([0x09]);
  });

  it('Up arrow emits ESC [ A', () => {
    expect(findBytes('↑')).toEqual([0x1b, 0x5b, 0x41]);
  });

  it('Down arrow emits ESC [ B', () => {
    expect(findBytes('↓')).toEqual([0x1b, 0x5b, 0x42]);
  });

  it('Left arrow emits ESC [ D', () => {
    expect(findBytes('←')).toEqual([0x1b, 0x5b, 0x44]);
  });

  it('Right arrow emits ESC [ C', () => {
    expect(findBytes('→')).toEqual([0x1b, 0x5b, 0x43]);
  });

  it('Enter emits 0x0d', () => {
    expect(findBytes('↵')).toEqual([0x0d]);
  });

  it('Ctrl+C emits 0x03', () => {
    expect(findBytes('^C')).toEqual([0x03]);
  });

  it('Ctrl+D emits 0x04', () => {
    expect(findBytes('^D')).toEqual([0x04]);
  });

  it('Slash emits 0x2f', () => {
    expect(findBytes('/')).toEqual([0x2f]);
  });

  it('all key labels are unique', () => {
    const labels = DEFAULT_KEYS.map((k) => k.label);
    const uniq = new Set(labels);
    expect(uniq.size).toBe(labels.length);
  });
});
