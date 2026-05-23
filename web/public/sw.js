// Claude Phone service worker.
//
// Goals:
//   - Make the static shell (HTML, JS, CSS, fonts, favicon, manifest)
//     available offline so the PWA can launch from the home screen on a
//     flaky connection.
//   - Never cache anything that contains a session token. That means
//     /s/<token> (the session shell HTML) and /api/phone/<token>
//     (the WebSocket upgrade) MUST be passthrough-only.
//   - Never cache the wrapper RPC, gateway WS, healthz, or anything else
//     under /api/*.

const CACHE_VERSION = 'cp-static-v1';

// Bumped each deploy; static assets are content-hashed by Vite so a fresh
// asset name appears on every release and stale caches get evicted on
// activation below.
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

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);

  if (!isCacheable(url)) {
    // Token-bearing or dynamic — go straight to the network.
    return;
  }

  // Stale-while-revalidate for static assets: serve from cache for instant
  // paint, then refresh the cache in the background so the next load picks
  // up new content.
  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_VERSION);
      const cached = await cache.match(req);
      const networkFetch = fetch(req)
        .then((resp) => {
          // Only cache successful, basic responses to avoid persisting error
          // pages or CDN miss responses.
          if (resp && resp.ok && resp.type === 'basic') {
            cache.put(req, resp.clone()).catch(() => {});
          }
          return resp;
        })
        .catch(() => undefined);

      if (cached) return cached;
      const fresh = await networkFetch;
      if (fresh) return fresh;
      // Both failed: synthesize a tiny response rather than letting the
      // browser show its offline error chrome for our own assets.
      return new Response('offline', { status: 503, statusText: 'offline' });
    })(),
  );
});
