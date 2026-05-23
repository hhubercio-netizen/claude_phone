import { useEffect, useState } from 'react';
import { useTerminal } from './useTerminal';
import styles from './Terminal.module.css';

export interface TerminalHandle {
  setFontSize: (px: number) => void;
  scrollToBottom: () => void;
}

export interface TerminalProps {
  onInputBytes: (bytes: Uint8Array) => void;
  onResize: (cols: number, rows: number) => void;
  /** Imperative handle for parent to push incoming bytes into xterm. */
  writeHandle?: (write: (bytes: Uint8Array) => void) => void;
  /** Optional handle exposing font-size + scroll controls to the parent. */
  controlHandle?: (h: TerminalHandle) => void;
  /** Initial font size in CSS pixels. */
  fontSize?: number;
}

export function Terminal({
  onInputBytes,
  onResize,
  writeHandle,
  controlHandle,
  fontSize,
}: TerminalProps) {
  const encoder = new TextEncoder();
  const [atBottom, setAtBottom] = useState(true);
  const term = useTerminal({
    onInput: (data) => onInputBytes(encoder.encode(data)),
    onResize,
    onScrollChange: setAtBottom,
    initialFontSize: fontSize,
  });

  useEffect(() => {
    if (!writeHandle) return;
    writeHandle((bytes) => term.write(bytes));
  }, [term, writeHandle]);

  useEffect(() => {
    if (!controlHandle) return;
    controlHandle({
      setFontSize: term.setFontSize,
      scrollToBottom: term.scrollToBottom,
    });
  }, [term, controlHandle]);

  // React to font-size prop changes (parent owns the value; xterm gets re-fitted
  // inside setFontSize).
  useEffect(() => {
    if (typeof fontSize === 'number') term.setFontSize(fontSize);
  }, [fontSize, term]);

  return (
    <div className={styles.wrapper} onClick={() => term.focus()}>
      <div ref={term.containerRef} className={styles.container} />
      {!atBottom && (
        <button
          type="button"
          className={styles.scrollBottom}
          onClick={(e) => {
            // Stop the click from also re-focusing the terminal — pressing
            // the pill is purely a viewport command, not an input event.
            e.stopPropagation();
            term.scrollToBottom();
          }}
          aria-label="Scroll to bottom"
        >
          ↓ live
        </button>
      )}
    </div>
  );
}
