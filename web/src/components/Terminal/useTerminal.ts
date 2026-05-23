import { useEffect, useRef, type RefObject } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
// xterm ships with a stylesheet that, among other things, moves the
// helper-textarea off-screen (left: -9999em). Without this import the
// textarea sits at the top-left of the terminal container with default
// browser styling — visible on mobile as a stray "(" / ")" / "*" glyph
// depending on what the OS keyboard/autocorrect renders, and catching any
// focus tap and dragging the viewport offscreen.
import '@xterm/xterm/css/xterm.css';
import { claudeTheme } from './theme';

export interface UseTerminalParams {
  onInput: (data: string) => void;
  onResize: (cols: number, rows: number) => void;
  /** Called whenever the scroll position changes; `atBottom` is true when the
      viewport sits flush against the latest output. Used to drive the
      scroll-to-bottom pill. */
  onScrollChange?: (atBottom: boolean) => void;
  /** Initial font size in CSS pixels. Default 13. */
  initialFontSize?: number;
}

export interface TerminalApi {
  containerRef: RefObject<HTMLDivElement>;
  write: (data: string | Uint8Array) => void;
  resize: () => void;
  focus: () => void;
  fit: FitAddon;
  scrollToBottom: () => void;
  setFontSize: (px: number) => void;
}

function computeAtBottom(term: XTerm): boolean {
  const buf = term.buffer.active;
  // viewportY = first visible row; baseY = first row of the scrollback region.
  // When viewportY >= baseY, the viewport is anchored to the live output.
  return buf.viewportY >= buf.baseY;
}

export function useTerminal({
  onInput,
  onResize,
  onScrollChange,
  initialFontSize = 13,
}: UseTerminalParams): TerminalApi {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const onInputRef = useRef(onInput);
  const onResizeRef = useRef(onResize);
  const onScrollChangeRef = useRef(onScrollChange);

  // keep latest handlers without re-init xterm
  onInputRef.current = onInput;
  onResizeRef.current = onResize;
  onScrollChangeRef.current = onScrollChange;

  useEffect(() => {
    if (!containerRef.current) return;
    const term = new XTerm({
      theme: claudeTheme,
      fontFamily: '"JetBrains Mono", Menlo, Consolas, monospace',
      fontSize: initialFontSize,
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
    term.onScroll(() => {
      onScrollChangeRef.current?.(computeAtBottom(term));
    });
    // Fire once so consumers start with the correct state.
    onScrollChangeRef.current?.(computeAtBottom(term));

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        /* term not attached yet */
      }
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
    // initialFontSize only seeds the initial render; later changes use setFontSize.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    containerRef,
    write: (data) => termRef.current?.write(data as never),
    resize: () => fitRef.current?.fit(),
    focus: () => termRef.current?.focus(),
    fit: fitRef.current!,
    scrollToBottom: () => termRef.current?.scrollToBottom(),
    setFontSize: (px) => {
      const term = termRef.current;
      if (!term) return;
      // Clamp to a sensible range. Below 10 is unreadable, above 22 fits
      // almost no useful content on a phone width.
      const clamped = Math.max(10, Math.min(22, Math.round(px)));
      term.options.fontSize = clamped;
      // Re-fit so cols/rows reflect the new cell size and the wrapper sees
      // an accurate resize.
      try {
        fitRef.current?.fit();
      } catch {
        /* not attached */
      }
    },
  };
}
