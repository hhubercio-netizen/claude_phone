import { describe, it, expect } from 'vitest';

// Vite's `?raw` query loads the file's text contents at build time. This
// means the tests reflect exactly what ships in the repo — a regression
// that adds a CDN script to index.html or removes the /api/* exclusion
// from sw.js fails the build, not just a dev-loop spot check.
import indexHtml from '../index.html?raw';
import swSource from '../public/sw.js?raw';

// TM-FRONT.10 / TM-SUPPLY.5 — single HTML shell, zero unhardened externals.
//
// `index.html` is the only HTML the site emits. Production ships content-
// hashed JS/CSS from `/assets/` (same-origin) and nothing else. If a
// future change adds a `<script src="https://cdn.example.com/lib.js">`
// without `integrity`+`crossorigin`, that's a supply-chain hole: a CDN
// compromise (or DNS hijack) gets to execute in the session origin with
// access to the SessionToken, paste contents, and the wrapper RPC.
//
// The tests below are forward-looking. They pass today because the only
// scripts/styles in `index.html` are same-origin Vite entry points. They
// will fail the moment someone adds an external dep without SRI, and the
// failure message points at this row of the threat model.
describe('TM-FRONT.10 / TM-SUPPLY.5: index.html has no unhardened externals', () => {
  const doc = new DOMParser().parseFromString(indexHtml, 'text/html');

  const isExternal = (urlAttr: string | null): boolean => {
    if (!urlAttr) return false;
    // Absolute http(s) or protocol-relative — anything not same-origin.
    return /^https?:\/\//i.test(urlAttr) || urlAttr.startsWith('//');
  };

  it('every external <script src> carries integrity + crossorigin (or none exist)', () => {
    const scripts = Array.from(doc.querySelectorAll('script[src]'));
    for (const s of scripts) {
      const src = s.getAttribute('src');
      if (!isExternal(src)) continue;
      expect(
        s.getAttribute('integrity'),
        `<script src="${src}"> is external — TM-SUPPLY.5 requires integrity=""`,
      ).toBeTruthy();
      expect(
        s.getAttribute('crossorigin'),
        `<script src="${src}"> is external — TM-SUPPLY.5 requires crossorigin=""`,
      ).toBeTruthy();
    }
  });

  it('every external stylesheet / modulepreload carries integrity + crossorigin', () => {
    const links = Array.from(
      doc.querySelectorAll(
        'link[rel~="stylesheet"][href], link[rel="modulepreload"][href], link[rel="preload"][href]',
      ),
    );
    for (const l of links) {
      const href = l.getAttribute('href');
      if (!isExternal(href)) continue;
      expect(
        l.getAttribute('integrity'),
        `<link href="${href}"> is external — TM-SUPPLY.5 requires integrity=""`,
      ).toBeTruthy();
      expect(
        l.getAttribute('crossorigin'),
        `<link href="${href}"> is external — TM-SUPPLY.5 requires crossorigin=""`,
      ).toBeTruthy();
    }
  });

  it('current shell has zero externals (red-team baseline)', () => {
    // The current ground truth: ZERO external scripts/styles. A future
    // refactor that adds even an integrity-protected external should also
    // surface here so it gets explicit review — the integrity-protected
    // path is OK, but it changes the supply-chain story and deserves a
    // commit-message paragraph. Bumping this expectation is the trigger.
    const scripts = Array.from(doc.querySelectorAll('script[src]')).filter((s) =>
      isExternal(s.getAttribute('src')),
    );
    const links = Array.from(
      doc.querySelectorAll(
        'link[rel~="stylesheet"][href], link[rel="modulepreload"][href], link[rel="preload"][href]',
      ),
    ).filter((l) => isExternal(l.getAttribute('href')));
    expect(scripts.length, 'no external <script src>').toBe(0);
    expect(links.length, 'no external <link href>').toBe(0);
  });

  it('TM-FRONT.10 marker is present in index.html as documentation anchor', () => {
    // The HTML comment that explains the zero-external posture must stay.
    // Without it the next contributor has no signal that "no external
    // scripts" is a deliberate security choice — they might think it's
    // accidental and add a CDN dep "for performance".
    expect(indexHtml).toContain('TM-FRONT.10');
  });
});

// TM-FRONT.4 — service worker MUST NOT cache token-bearing or /api/* paths.
//
// Caching `/s/<token>` puts the bearer token onto disk inside the browser's
// CacheStorage (no eviction guarantee, survives tab close). Caching `/api/*`
// would intercept WebSocket upgrade requests and gateway RPC, breaking
// liveness and potentially serving stale auth responses. The exclusion
// logic lives in `isCacheable()`; these tests pin both the structural
// shape (source string match) AND the behavior (function eval on test
// URLs) so neither cosmetic refactor nor logic regression goes silently.
describe('TM-FRONT.4: service worker excludes /api/* and /s/<token>', () => {
  it('source carries the TM-FRONT.4 anchor', () => {
    expect(swSource).toContain('TM-FRONT.4');
  });

  it('source contains the /api/ exclusion', () => {
    // Cosmetic-refactor brittleness is intentional. The catalog row is
    // load-bearing security and the operator hint relies on grep on
    // TM-FRONT.4 finding this exact code shape.
    expect(swSource).toMatch(/url\.pathname\.startsWith\(['"]\/api\/['"]\)\s*\)\s*return\s+false/);
  });

  it('source contains the /s/ exclusion', () => {
    expect(swSource).toMatch(/url\.pathname\.startsWith\(['"]\/s\/['"]\)\s*\)\s*return\s+false/);
  });

  it('source contains a cross-origin guard', () => {
    // First-line defense: same-origin only. Even if /api/ or /s/ logic
    // were to fail open, this would still keep an attacker-controlled
    // origin out of the cache.
    expect(swSource).toMatch(/url\.origin\s*!==\s*self\.location\.origin/);
  });

  it('isCacheable behavior pinned: rejects token-bearing and dynamic URLs', () => {
    // Extract the function body and re-evaluate it in test scope. `self`
    // is injected as a parameter so we can stub `self.location.origin`
    // without polluting jsdom's actual `self`.
    const match = swSource.match(/function\s+isCacheable\(url\)\s*\{([\s\S]*?)\n\}/);
    expect(match, 'isCacheable() shape must be greppable').toBeTruthy();
    const body = match![1];
    const fn = new Function('url', 'self', body) as (
      url: URL,
      self: { location: { origin: string } },
    ) => boolean;

    const ORIGIN = 'https://claude-phone.pl';
    const fakeSelf = { location: { origin: ORIGIN } };

    // Negative cases — must NOT cache:
    expect(fn(new URL(`${ORIGIN}/api/phone/abc`), fakeSelf), '/api/phone/<token>').toBe(false);
    expect(fn(new URL(`${ORIGIN}/api/wrapper`), fakeSelf), '/api/wrapper').toBe(false);
    expect(fn(new URL(`${ORIGIN}/s/abcdef0123`), fakeSelf), '/s/<token>').toBe(false);
    expect(fn(new URL(`${ORIGIN}/healthz`), fakeSelf), '/healthz').toBe(false);
    expect(fn(new URL('https://evil.example.com/x.js'), fakeSelf), 'cross-origin').toBe(false);

    // Positive cases — cacheable static shell + hashed assets:
    expect(fn(new URL(`${ORIGIN}/`), fakeSelf), 'root shell').toBe(true);
    expect(fn(new URL(`${ORIGIN}/assets/index-abc123.js`), fakeSelf), 'hashed asset').toBe(true);
    expect(fn(new URL(`${ORIGIN}/favicon.svg`), fakeSelf), 'favicon').toBe(true);
    expect(fn(new URL(`${ORIGIN}/manifest.webmanifest`), fakeSelf), 'manifest').toBe(true);
  });
});
