import { useEffect, useRef, useState } from 'react';
import styles from './PasteModal.module.css';

interface Props {
  open: boolean;
  onClose: () => void;
  onSend: (bytes: Uint8Array) => void;
}

// Hard cap on a single paste — must stay below the gateway's 64KB WS message
// cap with headroom for chunking, so we cap the textarea content at 32KB.
const MAX_PASTE_BYTES = 32 * 1024;

export function PasteModal({ open, onClose, onSend }: Props) {
  const [text, setText] = useState('');
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Focus textarea every time the modal is shown so the keyboard pops up
  // immediately on mobile.
  useEffect(() => {
    if (open) {
      // Defer focus until after layout: iOS Safari ignores focus() that
      // happens before the element is visible.
      const id = requestAnimationFrame(() => taRef.current?.focus());
      return () => cancelAnimationFrame(id);
    }
    // Reset content when closing — paste contents may contain anything the
    // user typed, so we drop it eagerly rather than persisting across opens.
    setText('');
    return;
  }, [open]);

  // Esc closes the modal. Mounted only while open so we don't compete with
  // the terminal's keyboard handling.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const encoder = new TextEncoder();
  const bytes = encoder.encode(text);
  const tooBig = bytes.byteLength > MAX_PASTE_BYTES;

  const send = () => {
    if (tooBig || bytes.byteLength === 0) return;
    onSend(bytes);
    onClose();
  };

  return (
    <div
      className={styles.backdrop}
      role="dialog"
      aria-modal="true"
      aria-label="Paste text"
      onClick={onClose}
    >
      <div
        className={styles.panel}
        onClick={(e) => e.stopPropagation()}
      >
        <div className={styles.header}>
          <span>Paste</span>
          <button
            type="button"
            className={styles.close}
            onClick={onClose}
            aria-label="Close paste modal"
          >
            ✕
          </button>
        </div>
        <textarea
          ref={taRef}
          className={styles.textarea}
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Type or paste, then Send."
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
        />
        <div className={styles.footer}>
          <span className={tooBig ? styles.warn : styles.muted}>
            {bytes.byteLength} / {MAX_PASTE_BYTES} bytes
          </span>
          <div className={styles.actions}>
            <button type="button" className={styles.btnGhost} onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className={styles.btnPrimary}
              onClick={send}
              disabled={tooBig || bytes.byteLength === 0}
            >
              Send
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
