import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ActionBar } from '../src/components/ActionBar/ActionBar';
import { DEFAULT_KEYS } from '../src/components/ActionBar/keys';

describe('ActionBar', () => {
  it('renders a button per key', () => {
    render(<ActionBar onKey={() => {}} />);
    for (const k of DEFAULT_KEYS) {
      expect(screen.getByRole('button', { name: k.label })).toBeInTheDocument();
    }
  });

  it('invokes onKey with the configured bytes on mousedown', () => {
    const onKey = vi.fn();
    render(<ActionBar onKey={onKey} />);
    const tabBtn = screen.getByRole('button', { name: 'Tab' });
    tabBtn.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(onKey).toHaveBeenCalledTimes(1);
    const bytes = onKey.mock.calls[0][0] as Uint8Array;
    expect(Array.from(bytes)).toEqual([0x09]);
  });

  it('uses provided custom keys instead of defaults', () => {
    const custom = [{ label: 'X', bytes: [0x58] }];
    render(<ActionBar onKey={() => {}} keys={custom} />);
    expect(screen.getByRole('button', { name: 'X' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Esc' })).not.toBeInTheDocument();
  });
});
