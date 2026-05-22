import { useEffect } from 'react';
import { useTerminal } from './useTerminal';
import styles from './Terminal.module.css';

export interface TerminalProps {
  onInputBytes: (bytes: Uint8Array) => void;
  onResize: (cols: number, rows: number) => void;
  /** Imperative handle for parent to push incoming bytes into xterm. */
  writeHandle?: (write: (bytes: Uint8Array) => void) => void;
}

export function Terminal({ onInputBytes, onResize, writeHandle }: TerminalProps) {
  const encoder = new TextEncoder();
  const term = useTerminal({
    onInput: (data) => onInputBytes(encoder.encode(data)),
    onResize,
  });

  useEffect(() => {
    if (!writeHandle) return;
    writeHandle((bytes) => term.write(bytes));
  }, [term, writeHandle]);

  return (
    <div
      ref={term.containerRef}
      className={styles.container}
      onClick={() => term.focus()}
    />
  );
}
