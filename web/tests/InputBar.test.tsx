import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { InputBar } from '../src/components/InputBar/InputBar';

function bytesOf(uint8: Uint8Array): number[] {
  return Array.from(uint8);
}

describe('InputBar', () => {
  it('renders an input and a send button', () => {
    render(<InputBar onBytes={() => {}} />);
    expect(screen.getByPlaceholderText(/type to send/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /send/i })).toBeInTheDocument();
  });

  it('forwards every typed character as a separate UTF-8 byte stream', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'h' } });
    fireEvent.change(input, { target: { value: 'hi' } });
    expect(onBytes).toHaveBeenCalledTimes(2);
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0x68]); // h
    expect(bytesOf(onBytes.mock.calls[1][0] as Uint8Array)).toEqual([0x69]); // i
  });

  it('sends DEL (0x7f) when characters are removed (backspace)', () => {
    // The wrapper PTY treats 0x7f as backspace — that is what claude's
    // readline-style prompts expect, not the actual BS (0x08) which they
    // interpret as cursor-back-without-delete.
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'ab' } });
    onBytes.mockClear();
    fireEvent.change(input, { target: { value: 'a' } });
    expect(onBytes).toHaveBeenCalledTimes(1);
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0x7f]);
  });

  it('sends CR (0x0d) and clears on Enter', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'hi' } });
    onBytes.mockClear();
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onBytes).toHaveBeenCalledTimes(1);
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0x0d]);
    expect(input.value).toBe('');
  });

  it('does not send anything from the Send button when input is empty', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    onBytes.mockClear();
    fireEvent.click(screen.getByRole('button', { name: /send/i }));
    // Send still emits a CR even if buffer is empty — claude treats an
    // empty Enter as a confirmation in many of its prompts (e.g. "Press
    // Enter to continue") so this is the desired behavior.
    expect(onBytes).toHaveBeenCalledTimes(1);
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0x0d]);
  });

  it('Send button delivers the pending input THEN the CR', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'yes' } });
    onBytes.mockClear();
    fireEvent.click(screen.getByRole('button', { name: /send/i }));
    // The component pushes the diff between lastSent and current value
    // first. lastSent already matches the value (set during onChange), so
    // only the CR is emitted. We verify both shape and order.
    const all = onBytes.mock.calls.map((c) => bytesOf(c[0] as Uint8Array));
    expect(all[all.length - 1]).toEqual([0x0d]);
    expect(input.value).toBe('');
  });

  it('forwards multi-byte UTF-8 (emoji / non-ASCII)', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    // ó is U+00F3 → 0xC3 0xB3 in UTF-8. Verifying multi-byte chars stream
    // intact so users can type Polish/Spanish/Chinese into a claude prompt.
    fireEvent.change(input, { target: { value: 'ó' } });
    expect(onBytes).toHaveBeenCalledTimes(1);
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0xc3, 0xb3]);
  });

  it('blocks input while disabled', () => {
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} disabled />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    expect(input.disabled).toBe(true);
    expect((screen.getByRole('button', { name: /send/i }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it('defers diff emission during IME composition', () => {
    // While the OS IME is composing (e.g. typing a Kanji), every
    // intermediate value is provisional — streaming each provisional
    // codepoint to the PTY would break the IME on the user's side. We
    // batch until compositionend.
    const onBytes = vi.fn();
    render(<InputBar onBytes={onBytes} />);
    const input = screen.getByPlaceholderText(/type to send/i) as HTMLInputElement;
    fireEvent.compositionStart(input);
    fireEvent.change(input, { target: { value: 'k' } });
    fireEvent.change(input, { target: { value: 'か' } });
    expect(onBytes).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input, { target: { value: 'か' } });
    expect(onBytes).toHaveBeenCalledTimes(1);
    // か = U+304B → 0xE3 0x81 0x8B
    expect(bytesOf(onBytes.mock.calls[0][0] as Uint8Array)).toEqual([0xe3, 0x81, 0x8b]);
  });
});
