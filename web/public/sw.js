// Claude Phone service worker.
//
// Goals:
//   - Make the static shell (HTML, JS, CSS, fonts, favicon, manifest)
//     available offline so the PWA can launch from the home screen on a
//     flaky connection.
//   - Never serve a stale HTML shell when the network is reachable.
//     The shell references content-hashed JS/CSS by exact filename, so a
//     stale shell pointing at deleted asset hashes produces a black screen
//     on the next deploy. Navigation requests therefore use network-first.
//   - Never cache anything that contains a session token. That means
//     /s/<token> (the session shell HTML) and /api/phone/<token>
//     (the WebSocket upgrade) MUST be passthrough-only.
//   - Never cache the wrapper RPC, gateway WS, healthz, or anything else
//     under /api/*.

// Bumped each deploy where the SW strategy changes. Vite content-hashes
// JS/CSS so individual asset cache keys evict naturally as new builds
// reference new hashes; this version bump is what evicts the previous
// HTML shell which would otherwise be served stale-while-revalidate
// forever and reference deleted asset hashes.
const CACHE_VERSION = 'cp-static-v3';

const PRECACHE_URLS = ['/', '/manifest.webmanifest', '/favicon.svg'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE_VERSION).then((cache) =>
      // Precache best-effort: a missing entry shouldn't block the install
      // (e.g. when the page is opened from a non-root path the very first time).
      Promise.allSettled(PRECACHE_URLS.map((u) => cache.add(u))),
    ),
  );
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys.filter((k) => k !== CACHE_VERSION).map((k) => caches.delete(k)),
      );
      await self.clients.claim();
    })(),
  );
});

function isCacheable(url) {
  // Only cache GETs on our own origin.
  if (url.origin !== self.location.origin) return false;
  // /api/* is the gateway. Never cache it.
  if (url.pathname.startsWith('/api/')) return false;
  // /s/<token> embeds the bearer-equivalent token in the URL. Never cache it.
  if (url.pathname.startsWith('/s/')) return false;
  // /healthz changes with server state; passthrough.
  if (url.pathname === '/healthz') return false;
  return true;
}

// Content-hashed asset paths emitted by Vite (e.g. /assets/index-ElAQ2Vvc.js).
// Filename includes an immutable build hash so cache-first is always safe —
// a new build emits a new filename and the old one is fine to evict lazily.
function isHashedAsset(url) {
  return url.pathname.startsWith('/assets/');
}

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);

  if (!isCacheable(url)) {
    // Token-bearing or dynamic — go straight to the network.
    return;
  }

  // Navigation requests (the HTML shell) MUST be network-first. The shell
  // references content-hashed JS/CSS by exact filename; a stale shell can
  // point at hashes that no longer exist on the server, which renders as
  // a blank page on the next deploy. Fall back to cache only when offline.
  if (req.mode === 'navigate') {
    event.respondWith(
      (async () => {
        try {
          const fresh = await fetch(req);
          if (fresh && fresh.ok && fresh.type === 'basic') {
            const cache = await caches.open(CACHE_VERSION);
            cache.put(req, fresh.clone()).catch(() => {});
          }
          return fresh;
        } catch {
          const cache = await caches.open(CACHE_VERSION);
          const cached = await cache.match(req);
          if (cached) return cached;
          // Try the root shell as a SPA fallback before giving up.
          const root = await cache.match('/');
          if (root) return root;
          return new Response('offline', { status: 503, statusText: 'offline' });
        }
      })(),
    );
    return;
  }

  // Hashed assets are immutable for a given filename — cache-first is the
  // fast path and only misses populate the cache.
  if (isHashedAsset(url)) {
    event.respondWith(
      (async () => {
        const cache = await caches.open(CACHE_VERSION);
        const cached = await cache.match(req);
        if (cached) return cached;
        try {
          const fresh = await fetch(req);
          if (fresh && fresh.ok && fresh.type === 'basic') {
            cache.put(req, fresh.clone()).catch(() => {});
          }
          return fresh;
        } catch {
          return new Response('offline', { status: 503, statusText: 'offline' });
        }
      })(),
    );
    return;
  }

  // Everything else (favicon, manifest) — stale-while-revalidate is fine.
  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_VERSION);
      const cached = await cache.match(req);
      const networkFetch = fetch(req)
        .then((resp) => {
          if (resp && resp.ok && resp.type === 'basic') {
            cache.put(req, resp.clone()).catch(() => {});
          }
          return resp;
        })
        .catch(() => undefined);

      if (cached) return cached;
      const fresh = await networkFetch;
      if (fresh) return fresh;
      return new Response('offline', { status: 503, statusText: 'offline' });
    })(),
  );
});
