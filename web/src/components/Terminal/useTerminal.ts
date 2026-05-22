import { useEffect, useRef } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { claudeTheme } from './theme';

export interface UseTerminalParams {
  onInput: (data: string) => void;
  onResize: (cols: number, rows: number) => void;
}

export interface TerminalApi {
  containerRef: React.RefObject<HTMLDivElement>;
  write: (data: string | Uint8Array) => void;
  resize: () => void;  // ask the fit addon to recompute size
  focus: () => void;
  fit: FitAddon;
}

export function useTerminal({ onInput, onResize }: UseTerminalParams): TerminalApi {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const onInputRef = useRef(onInput);
  const onResizeRef = useRef(onResize);

  // keep latest handlers without re-init xterm
  onInputRef.current = onInput;
  onResizeRef.current = onResize;

  useEffect(() => {
    if (!containerRef.current) return;
    const term = new XTerm({
      theme: claudeTheme,
      fontFamily: '"JetBrains Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      cursorBlink: true,
      convertEol: false,
      allowProposedApi: true,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    fit.fit();

    term.onData((d) => onInputRef.current(d));
    term.onResize(({ cols, rows }) => onResizeRef.current(cols, rows));

    const ro = new ResizeObserver(() => {
      try { fit.fit(); } catch { /* term not attached yet */ }
    });
    ro.observe(containerRef.current);

    termRef.current = term;
    fitRef.current = fit;
    return () => {
      ro.disconnect();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, []);

  return {
    containerRef,
    write: (data) => termRef.current?.write(data as never),
    resize: () => fitRef.current?.fit(),
    focus: () => termRef.current?.focus(),
    fit: fitRef.current!,
  };
}
