import { useRef, useState } from 'react';
import styles from './InputBar.module.css';

interface Props {
  /** Push raw bytes to the wrapper (same path as ActionBar / xterm onData). */
  onBytes: (bytes: Uint8Array) => void;
  /** Disable typing when the WebSocket isn't open or wrapper isn't paired. */
  disabled?: boolean;
}

// A mobile-first composer that replaces relying on xterm's hidden helper
// textarea for keyboard input. The user types here, every character
// (including the newline) is forwarded to the PTY in real time — claude's
// TUI echoes it back which appears in the terminal area above.
//
// Why a separate input instead of focusing xterm:
//   1. On iOS Safari, focusing xterm's offscreen helper-textarea makes the
//      browser scroll the page to surface that 1×1 element, dragging the
//      whole UI off-screen ("black screen no way back").
//   2. The mobile soft keyboard plays better with a real <input>: enterKeyHint,
//      autocomplete controls, and the OS-native dismissal gesture all work.
//   3. Touch scroll on the terminal area is no longer competing with an
//      onClick handler that wants to refocus xterm.
export function InputBar({ onBytes, disabled }: Props) {
  const [value, setValue] = useState('');
  const composingRef = useRef(false);
  const lastSentRef = useRef('');
  const encoder = new TextEncoder();

  // Forward the diff between previous value and new value as bytes — added
  // characters get sent as their UTF-8 encoding, removed characters trigger
  // backspace (0x7f, DEL — what claude's readline-style prompts expect).
  // Composition (IME) batches are deferred until composition ends so a
  // half-typed kanji/diacritic doesn't stream byte-by-byte.
  function pushDiff(next: string) {
    if (composingRef.current) return;
    const prev = lastSentRef.current;
    if (next === prev) return;

    // Common prefix length — anything before this index is unchanged.
    let i = 0;
    while (i < prev.length && i < next.length && prev[i] === next[i]) i++;

    const removed = prev.length - i;
    const added = next.slice(i);
    for (let k = 0; k < removed; k++) {
      onBytes(new Uint8Array([0x7f])); // DEL
    }
    if (added.length > 0) {
      onBytes(encoder.encode(added));
    }
    lastSentRef.current = next;
  }

  function handleChange(e: React.ChangeEvent<HTMLInputElement>) {
    const next = e.target.value;
    setValue(next);
    pushDiff(next);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      // Flush any pending diff first (shouldn't happen since onChange runs
      // before keydown for the same keystroke, but defensive).
      pushDiff(value);
      onBytes(new Uint8Array([0x0d])); // CR — what a TTY enter sends
      setValue('');
      lastSentRef.current = '';
    }
  }

  function handleSend() {
    pushDiff(value);
    onBytes(new Uint8Array([0x0d]));
    setValue('');
    lastSentRef.current = '';
  }

  return (
    <div className={styles.bar}>
      <input
        type="text"
        className={styles.input}
        placeholder="Type to send..."
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        onCompositionStart={() => {
          composingRef.current = true;
        }}
        onCompositionEnd={(e) => {
          composingRef.current = false;
          pushDiff((e.target as HTMLInputElement).value);
        }}
        disabled={disabled}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        inputMode="text"
        enterKeyHint="send"
        aria-label="Type to send to claude"
      />
      <button
        type="button"
        className={styles.send}
        onClick={handleSend}
        disabled={disabled}
        aria-label="Send (Enter)"
      >
        Send
      </button>
    </div>
  );
}
