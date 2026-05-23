import { useCallback, useEffect, useState } from 'react';

const STORAGE_KEY = 'cp.fontSize';
const MIN = 10;
const MAX = 22;
const DEFAULT = 13;

function clamp(n: number): number {
  return Math.max(MIN, Math.min(MAX, Math.round(n)));
}

function readStored(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw == null) return DEFAULT;
    const n = Number.parseInt(raw, 10);
    if (!Number.isFinite(n)) return DEFAULT;
    return clamp(n);
  } catch {
    // localStorage can throw in private mode / disabled storage. Fall back
    // to the in-memory default so the UI still works.
    return DEFAULT;
  }
}

/**
 * Persisted font-size state for the terminal. Stores only the integer pixel
 * size (NOT the token or anything session-bound), so no leakage risk.
 */
export function useFontSize() {
  const [size, setSize] = useState<number>(() => readStored());

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(size));
    } catch {
      /* ignore storage failures */
    }
  }, [size]);

  const inc = useCallback(() => setSize((s) => clamp(s + 1)), []);
  const dec = useCallback(() => setSize((s) => clamp(s - 1)), []);
  const reset = useCallback(() => setSize(DEFAULT), []);

  return { size, inc, dec, reset, min: MIN, max: MAX, default: DEFAULT };
}
