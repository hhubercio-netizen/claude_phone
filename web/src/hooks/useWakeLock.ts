import { useEffect } from 'react';

// Minimal subset of the Wake Lock API we use. Avoids a hard dependency on
// `lib.dom.iterable` types that aren't shipped in all TS configurations.
interface WakeLockSentinelLike {
  released: boolean;
  release(): Promise<void>;
  addEventListener(type: 'release', listener: () => void): void;
}

interface WakeLockApi {
  request(type: 'screen'): Promise<WakeLockSentinelLike>;
}

function getWakeLock(): WakeLockApi | null {
  // navigator.wakeLock is undefined on older browsers and on insecure contexts.
  const nav = navigator as unknown as { wakeLock?: WakeLockApi };
  return nav.wakeLock ?? null;
}

/**
 * Hold a screen wake lock while `active` is true. The browser auto-releases
 * the lock whenever the tab is hidden, so we re-acquire on visibilitychange.
 * No-ops cleanly on platforms without the Wake Lock API (older Safari, etc.).
 */
export function useWakeLock(active: boolean) {
  useEffect(() => {
    if (!active) return;
    const api = getWakeLock();
    if (!api) return;

    let sentinel: WakeLockSentinelLike | null = null;
    let cancelled = false;

    const acquire = async () => {
      try {
        const s = await api.request('screen');
        if (cancelled) {
          // Effect already torn down — release immediately and bail.
          try {
            await s.release();
          } catch {
            /* ignore */
          }
          return;
        }
        sentinel = s;
        s.addEventListener('release', () => {
          // The browser may release on tab hide; clear our handle so the
          // next visibility-visible re-acquires fresh.
          if (sentinel === s) sentinel = null;
        });
      } catch {
        // Wake Lock requests can reject (e.g. user setting, low battery,
        // page not focused). Treat as best-effort; nothing else to do.
      }
    };

    const onVisibility = () => {
      if (document.visibilityState === 'visible' && !sentinel) {
        void acquire();
      }
    };

    void acquire();
    document.addEventListener('visibilitychange', onVisibility);

    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', onVisibility);
      if (sentinel) {
        void sentinel.release().catch(() => {});
        sentinel = null;
      }
    };
  }, [active]);
}
