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

  // No onClick={term.focus()} here on purpose. The terminal is display-only
  // on mobile — typing happens in the dedicated InputBar below, so touching
  // the terminal area must remain a pure scroll surface. Focusing xterm
  // here would re-introduce the iOS scroll-into-view-for-helper-textarea
  // bug that drags the entire UI off-screen.
  return (
    <div className={styles.wrapper}>
      <div ref={term.containerRef} className={styles.container} />
      {!atBottom && (
        <button
          type="button"
          className={styles.scrollBottom}
          onClick={() => term.scrollToBottom()}
          aria-label="Scroll to bottom"
        >
          ↓ live
        </button>
      )}
    </div>
  );
}
