# Security Hardening Design — Claude Phone (Master Spec)

**Status:** Draft (brainstorming 2026-05-23) — pending user approval (Stage 1 gate)
**Companion:** `2026-05-23-threat-model.md` (authoritative threat list + `TM-CAT.N` catalog)
**Branch policy:** direct-to-`main`, push to `origin/main` after each major chunk
**Implementation:** see per-category sub-specs `2026-05-23-sec-4.X-*.md` once Stage 2 approved
**Supersedes:** ad-hoc hardening notes in `docs/security.md` (v1 will be reduced to a pointer)

---

## 1. Cross-cutting principles

These six principles bind every sub-spec and every code change in this hardening
pass. They are the standard against which Stage 4 (execution) is measured.

### 1.1 Defense-in-depth

Every protection has a backup. Origin checks **plus** ct-eq auth **plus** rate
limit **plus** fail2ban. A single bug in any layer does not lose the asset. In
practice: when a mitigation is removed during refactor, at least one other layer
remains; tests assert this layered behaviour.

### 1.2 Zero-trust at every boundary

Each trust boundary (T1-T7 in threat model §2.2) re-validates everything. The
wrapper does not trust the gateway to have authorized the phone — wherever it
sees a token, it verifies. The gateway does not trust Caddy to have terminated
TLS — it still emits HSTS on its own responses. No mitigation relies on "the
caller will have checked already".

### 1.3 Fail-loud, fail-closed

Every error in an auth or trust path → `deny + structured log + return error`,
never silent fallback. Config-load failures abort startup. Token unparseable →
401 (not 500, not 200-with-empty-body). When `public_origin` is set, missing
Origin header fails closed (`TM-WS.3`). Constants that have no safe default
(api_key, public_origin) fail at startup, not at runtime.

### 1.4 Audit-ready

Every security-relevant event (auth failure, sweeper drop, registration
collision, rate-limit trigger) emits a structured log line with a correlation
ID and no secret payload. Logs are sufficient to reconstruct an incident
without leaking the credential. Phase 2 sub-spec 4.7 adds the auth-failure
audit log (`TM-AUTH.7`); 4.6 adds the rate-limit-trigger log.

### 1.5 Threat-model-driven

No guard exists "just in case". Every guard in code has a `// TM-CAT.N:
<reason>` comment referencing the threat model. Conversely, every `TM-CAT.N`
in the catalog has either a mitigation or an explicit "Accepted risk" line in
threat model §9. The TM coverage matrix (§8 of this spec) is mechanically
checked in CI (`TM-LEAK.4`).

### 1.6 Forward-looking tests

Every fix ships with a test that **breaks if a future change re-opens the
hole**. Tests defend invariants (e.g. "no tracing macro takes `?token`"), not
behaviours. Three reinforcing layers per mitigation: test + code comment + CI
grep. Tests can be deleted, comments ignored, grep silenced — but all three
together is sticky.

These are restated verbatim in commit messages where applicable so future
maintainers see the principle, not just the patch.

---

## 2. Scope summary

### 2.1 In-scope categories (15 from `prompt-claude.txt §4`)

Categories 4.2 through 4.15 from the source brief are within this hardening
pass. Each is either a P0/P1 dedicated sub-spec or a P2 inline mini-section
in this document (§4 below). The classification follows threat model §10:

- **P0** (Stage 4 first wave): 4.2 (auth), 4.3 (transport), 4.5 (input),
  4.6 (rate), 4.7 (secrets), 4.9 (infra), 4.13 (ws), 4.14 (code-audit).
  Direct link to top-3 assets or actor capability.

- **P1** (Stage 4 second wave): 4.15 (testing). Cross-cuts P0; validates them
  after they land. Cannot fully exist until P0 mitigations are wired.

- **P2** (this spec, inline): 4.4 (browser headers — mostly done), 4.8
  (supply chain — mostly done, `cargo-deny` only), 4.10 (ops), 4.11
  (privacy / RODO), 4.12 (frontend deferrals).

### 2.2 Out of scope (recap from threat model §9)

18 items, see threat model §9 for full list. Highlights:

- Physical access, Cloudflare account compromise, registrar compromise,
  Anthropic API key (user's responsibility).
- `cargo-vet`, reproducible-build signing, formal SECURITY.md with PGP +
  bug bounty (private repo + small user base ⇒ overkill).
- mTLS for wrapper↔gateway, COOP/COEP, Trusted Types, nonce-based style-src
  (all P2 deferrals — current CSP is strict enough at this scale).
- HSTS preload registry submission (manual ops step after 30 d clean).

If during execution a new category surfaces that is not covered here, it is
out-of-scope until explicitly added by user. No silent scope creep.

### 2.3 Existing baseline (do not regress)

Threat model §12 catalogs Round 1–3 mitigations already in place. The
**pre-step regression sweep** policy in §7 below requires verification of
this baseline before every sub-spec lands. Failures block the merge.

---

## 3. Sub-spec index (P0/P1, full documents)

Each sub-spec is a self-contained design (~150-300 lines): scope, listed
`TM-CAT.N` items it covers, concrete changes per file, test plan with named
forward-looking tests, commit boundaries, and risk-of-regression notes.

| #    | Sub-spec                             | File                                          | Covers `TM-CAT.N`                                                                                          | Depends on                              |
|------|--------------------------------------|-----------------------------------------------|------------------------------------------------------------------------------------------------------------|-----------------------------------------|
| P0-1 | Authentication & Session             | `2026-05-23-sec-4.2-auth.md`                  | TM-AUTH.3, .4, .6, .7; TM-INFRA.4 (via auth-failure log)                                                    | none                                    |
| P0-2 | Transport Security                   | `2026-05-23-sec-4.3-transport.md`             | TM-TLS.1, .4, .6, .7, .8; TM-WS.9                                                                          | none                                    |
| P0-3 | Input Validation & Output Encoding   | `2026-05-23-sec-4.5-input.md`                 | TM-INPUT.1-8; TM-WS.4/5 (verify regression)                                                                | none                                    |
| P0-4 | Rate Limiting & DoS                  | `2026-05-23-sec-4.6-rate-limit.md`            | TM-RATE.1-9; TM-WS.7, .10                                                                                  | P0-1 (auth-failure log feeds rate-limit)|
| P0-5 | Secrets Management                   | `2026-05-23-sec-4.7-secrets.md`               | TM-SECRET.1, .8, .10, .11, .13; TM-LEAK.1, .2; TM-SUPPLY.5 (verify SRI)                                    | none                                    |
| P0-6 | Infrastructure Hardening             | `2026-05-23-sec-4.9-infra.md`                 | TM-INFRA.1-11; TM-RATE.5 (LimitNOFILE)                                                                     | none                                    |
| P0-7 | WebSocket-specific                   | `2026-05-23-sec-4.13-websocket.md`            | TM-WS.3, .7, .8, .9, .10, .12; TM-LEAK.3                                                                   | P0-3 (input limits)                     |
| P0-8 | Code-Level Audit                     | `2026-05-23-sec-4.14-code-audit.md`           | TM-CODE.1-7; TM-SUPPLY.4 (cargo-deny config)                                                               | none                                    |
| P1-1 | Testing & Verification               | `2026-05-23-sec-4.15-testing.md`              | TM-TEST.1-6; TM-LEAK.1-4 (CI scripts)                                                                      | all P0 (validates them)                 |

### 3.1 Ordering rationale

P0 sub-specs are independent at design level (no design depends on another).
At **execution** level (Stage 4), P0-4 should land after P0-1 (it consumes
the auth-failure log) and P0-7 should land after P0-3 (it tightens WS limits
that depend on input policy). All other P0 ordering is free; default order
in the writing-plans output will be 4.2 → 4.5 → 4.7 → 4.6 → 4.13 → 4.3 →
4.9 → 4.14, with 4.15 last.

### 3.2 Mitigation IDs and code comments

Every change a sub-spec produces in code carries `// TM-CAT.N: <one-line
reason>` immediately above the guard / config / check. This comment is part
of the guard, not decoration. Removing the guard removes the comment;
removing only the comment is caught by `TM-LEAK.4` coverage matrix.

---

## 4. P2 inline mini-sections

These categories are P2 because either: most items are already GREEN, or
the residual delta is small / deferred-minimal. Each section below is the
**entire spec** for that category — there is no sub-spec file.

### 4.4 Browser Security Headers (mini-spec)

**Status:** mostly GREEN. Round 2 (commit `7d1fd1d`) installed CSP, HSTS,
Permissions-Policy, X-Frame-Options DENY, X-Content-Type-Options nosniff,
Referrer-Policy no-referrer, hidden Server header. Verified GREEN in
2026-05-23 sweep (`crates/claude-phone-gateway/src/http.rs:91-138`).

**P2 work in this pass:**

- **Verify uniform application across response types** — add unit/integration
  test asserting CSP and HSTS appear on 404, 500, redirect, OPTIONS, and
  upgrade-failed responses. The current `SetResponseHeaderLayer` is global
  per the router build, but a wrong middleware order can shadow it. Test
  goes in `crates/claude-phone-gateway/tests/headers_test.rs`.

- **Document deferred items** as `TM-CAT.N` entries with **Accepted risk
  (deferred P2)** annotation in threat model §13:
  - `TM-FRONT.1` nonce-based `style-src` (current: `'self' 'unsafe-inline'`).
    Rationale: xterm.js + Vite emit inline styles for cursor positioning;
    nonce implementation requires Vite plugin + per-response nonce
    generation. P2 because CSP is otherwise `default-src 'self'` and the
    same-origin-only frontend has no untrusted script surface.
  - `TM-FRONT.12` COOP `same-origin` and `TM-FRONT.13` COEP `require-corp`
    — strengthens isolation for SharedArrayBuffer / Spectre side-channel.
    P2 because we do not use SharedArrayBuffer and the surface is small.

- **No new code; no new tests beyond `headers_test.rs`.** Master spec
  marks `TM-FRONT.1`, `TM-FRONT.12`, `TM-FRONT.13` as **DEFER (P2)** in
  threat model §13.

**Commit boundary:** single commit `security(4.4): assert headers on all
response types`. References `TM-FRONT.6` through `TM-FRONT.9`.

### 4.8 Supply Chain Security (mini-spec)

**Status:** mostly GREEN. `cargo-audit`, `npm audit`, Dependabot weekly
(cargo + npm + github-actions) all in place (commit `7d1fd1d`). Verified
GREEN in 2026-05-23 sweep (`.github/workflows/ci.yml`,
`.github/dependabot.yml`).

**P2 work in this pass:**

- **Add `cargo-deny` configuration** (`TM-SUPPLY.4`). Single file
  `deny.toml` at workspace root. Three policies:
  - `[licenses]`: allowlist (`MIT`, `Apache-2.0`, `BSD-3-Clause`,
    `BSD-2-Clause`, `ISC`, `Unicode-DFS-2016`, `MPL-2.0`); deny others
    (especially GPL/AGPL contamination).
  - `[advisories]`: `vulnerability = "deny"`, `yanked = "deny"`,
    `unmaintained = "warn"`, `unsound = "deny"`.
  - `[bans]`: deny multiple-versions for `tokio`, `axum`, `hyper`
    (force lockfile consolidation).
  - `[sources]`: only `crates.io`; no git deps allowed without explicit
    allowlist (currently empty).
- CI step in `.github/workflows/ci.yml` running `cargo deny check`
  (single new step in `cargo-audit` job).

**Tests / regression catch:**

- `cargo deny check` is itself the test (CI gate).
- No source code change; no Rust-level test.

**Out-of-scope (deferred):** `cargo-vet` manual transitive-dep review
(too much overhead for a small project; documented as Accepted risk in
threat model §9.6). Reproducible builds / cosign signing (§9.17).

**Commit boundary:** single commit `security(4.8): add cargo-deny config
and CI step`. References `TM-SUPPLY.4`.

### 4.10 Operational Security (mini-spec)

**Status:** minimal baseline (`/healthz` exists). Most ops items in
prompt §4.10 are out of scope for a private-repo small-user-base project
(external log shipping, status page, alerting infrastructure).

**P2 work in this pass:** two documents only.

- **`SECURITY.md` at repo root** — short policy (≤ 30 lines):
  - "This is a private repository. Vulnerability disclosure: email to
    `<placeholder>` — replace before commit."
  - Expected response time: 7 d acknowledgement, 30 d fix or accepted-risk.
  - No PGP key, no bug bounty (out of scope per threat model §9.13).

- **`docs/INCIDENT_RESPONSE.md`** — runbook covering the 5 scenarios from
  prompt §4.10:
  1. API key leak (rotate via `revoked.toml` SIGHUP — `TM-AUTH.4`).
  2. Wrapper-host compromise (invalidate all api_keys for that host,
     check tracing for last-seen sessions).
  3. DDoS (CF rules toggle steps, ufw deny-from steps).
  4. Unauthorized phone session (sweep sessions, force-cycle api_keys for
     suspected users).
  5. Cloudflare account compromise (DNS lock, rotate API tokens,
     reissue cert).

- **`/healthz` endpoint** — already GREEN, no change.

- **Off-host log shipping** — out of scope. Local systemd journal +
  `journalctl --persistent` only (sub-spec 4.9 covers persistence).

- **Status page / external alerting** — out of scope at this scale.

**Tests / regression catch:**

- `SECURITY.md` existence test in CI (one-line `test -f SECURITY.md`).
- `docs/INCIDENT_RESPONSE.md` existence test (same pattern).
- No code-level test.

**Commit boundary:** single commit `security(4.10): SECURITY.md +
INCIDENT_RESPONSE.md runbook`. No `TM-CAT.N` reference (this is process,
not threat-model-direct).

### 4.11 Privacy & GDPR (mini-spec, deferred-minimal)

**Status:** the project is single-domain (`claude-phone.pl`) under Polish
jurisdiction → RODO/GDPR applies. Privacy footprint is small (no
analytics, no cookies, no third-party scripts) but a written policy is due
diligence.

**P2 work in this pass:** one HTML page + no procedural backbone.

- **`web/public/privacy.html`** — bilingual single-page policy (PL + EN).
  Content scope (threat model §8 covers the data inventory; this file
  is the user-facing version):
  - Data we receive: IP, User-Agent (CF + gateway), session token (URL
    bar until `history.replaceState`), api_key (server-side only).
  - Retention: CF 24 h default, gateway tracing 7 d, Caddy 7 d.
  - No analytics, no third-party scripts, no advertising cookies. CF may
    set `__cf_bm` for bot management — disclose.
  - Contact email for erasure requests (placeholder, must be set before
    deploy).
  - DPO statement: **not required** at this scale (small private project,
    no large-scale processing) — single-line note.
- **Footer link** in web UI to `/privacy.html` (one `<a>` element in
  the app shell).
- **Right-to-erasure procedure** — informal admin steps in
  `docs/INCIDENT_RESPONSE.md` (created in §4.10): admin finds api_key
  rows in `gateway.toml`, removes, SIGHUP gateway, grep tracing for
  last-7-d IP/UA pairs and rotate keys for users on that IP.

**Cookie policy:** "this site sets no cookies of its own; Cloudflare may
set `__cf_bm` for bot management" — embedded in privacy.html, no banner.

**Tests / regression catch:**

- `web/public/privacy.html` existence test in CI.
- Web e2e: footer link to `/privacy.html` resolves 200.
- No Rust code change.

**Commit boundary:** single commit `security(4.11): privacy.html PL+EN +
footer link`. No `TM-CAT.N` reference.

### 4.12 Frontend-Specific Hardening (mini-spec)

**Status:** Round 2 covered SW scope, `frame-ancestors 'none'`,
Permissions-Policy. Leakage tests (`2026-05-22-test-coverage`) cover
localStorage / sessionStorage / history-state. Verified GREEN in
2026-05-23 sweep.

**P2 work in this pass:**

- **`history.replaceState()` to clean token from URL bar** (`TM-FRONT.3`).
  After first WebSocket connect succeeds, replace URL with `/s/` (no
  token). Defends against C2 (browser history capture) and casual
  shoulder-surfing. Add to `web/src/main.tsx` post-`connect()` callback.

- **`autocomplete="off"`** on any input fields (currently none used for
  auth, but font-size selector and future settings panel — `TM-FRONT.11`).
  One-line attribute on each input.

- **xterm.js OSC handler audit** (`TM-INPUT.4`) — partially overlaps with
  sub-spec 4.5. Mini-section delegates client-side OSC/DCS disabling to
  4.5 (`disable OSC 52, OSC 8` in xterm.js parser registration). Master
  spec confirms this overlap; no duplicate work.

- **SW scope verification** (`TM-FRONT.4`) — add unit test for SW
  bypass on `/api/*` and `/s/<token>` (currently relies on inline
  comment in `web/src/sw.ts`).

- **Deferred to threat model §9:**
  - `TM-FRONT.1` nonce-based style — Vite plugin overhead; current CSP
    is strict enough. Documented Accepted risk.
  - `TM-FRONT.2` Trusted Types — current attack surface zero
    (no innerHTML usage outside xterm.js, which manages its own DOM).
    Documented Accepted risk.
  - `TM-FRONT.12/.13` COOP / COEP — no SharedArrayBuffer use.
    Documented Accepted risk.

**Tests / regression catch:**

- `web/src/leakage.test.ts` (existing) covers FRONT.5.
- New test: `expect(window.location.pathname).toMatch(/^\/s\/?$/)` after
  connect (covers `TM-FRONT.3`).
- New test: SW respondWith bypass for `/api/*`, `/s/<token>` (covers
  `TM-FRONT.4`).

**Commit boundary:** single commit `security(4.12): replaceState token
clean + autocomplete=off + SW bypass test`. References `TM-FRONT.3`,
`TM-FRONT.4`, `TM-FRONT.11`.

---

## 5. Cross-cutting test policy

This applies to **every** sub-spec without restatement.

### 5.1 Test-first (TDD per sub-spec task)

Each task in a sub-spec follows: write the test, see it fail (red), make it
pass (green), commit. No test ships separately from its mitigation; no
mitigation ships without a paired test. Forward-looking tests (§5.3) may
be added in a follow-up commit if the mitigation is purely behavioural
(e.g. a header value), but the behavioural test is mandatory in the same
commit as the change.

### 5.2 Three-layer reinforcement (per mitigation)

Threat model §7.4 defines the contract. Restated for the spec:

1. **Behavioural test** — does the mitigation work? (positive + negative.)
2. **Forward-looking invariant test** — does it stay working if someone
   refactors? Asserts the invariant, not the implementation.
3. **CI grep / code comment** — `// TM-CAT.N: <reason>` at the guard site;
   `scripts/security_invariants.sh` step in CI catches the mechanical
   regressions (tracing-with-secret, derived-Debug-with-secret).

### 5.3 Forward-looking invariant test catalogue (template)

Each sub-spec lists invariant tests in a table. Example shape (each
sub-spec fills its own rows):

| Invariant                                                    | Test file                                          | Test name                                                |
|--------------------------------------------------------------|----------------------------------------------------|-----------------------------------------------------------|
| no tracing macro takes `?token` / `?api_key`                 | `scripts/security_invariants.sh`                   | grep step (CI)                                            |
| `PairResponse` Debug never contains real token               | `crates/claude-phone-wrapper/tests/leakage_test.rs`| `pair_response_debug_does_not_leak_token`                 |
| Origin check fail-closed when public_origin set + Origin missing | `crates/claude-phone-gateway/tests/origin_test.rs` | `phone_ws_rejects_missing_origin_when_origin_configured`  |

### 5.4 No `#[allow(...)]` to dodge tests / clippy

Per user constraint (prompt §7). If a clippy lint trips, the fix is to
correct the code, not to silence the lint. If a test would have to be
disabled to pass another test, the fix is to redesign the test, not to
delete or `#[ignore]` it.

### 5.5 `cargo test --workspace` and `npm -w web test` must pass before push

Per user constraint (prompt §6). Pre-commit hooks in sub-spec 4.7
include a fast subset (clippy `-D warnings` + unit tests); full
`--workspace` runs in CI.

### 5.6 No `--no-verify` to bypass hooks

Per user constraint (prompt §7). Hook failures are root-caused and fixed.

---

## 6. Cross-cutting commit policy

This applies to **every** commit in this hardening pass.

### 6.1 Commit message format

```
security(<sub-spec>): <verb> <object>

<2-4 line body explaining WHY, referencing TM-CAT.N if applicable.>

Mitigates: TM-CAT.N[, TM-CAT.N…]
```

Sub-spec slot is `4.X` from prompt §4 or `lower-case-category` for
P2 mini-sections. Verb is imperative (`add`, `tighten`, `reject`,
`enforce`, `rotate`, `harden`). Footer with `Mitigates:` is mandatory
when the commit touches a `TM-CAT.N`-tracked guard.

### 6.2 Atomic commits per sub-spec task

Each task (defined in sub-spec) is one commit. Tasks defined by:

- **One concern per commit** — one guard, one config, one test pair.
  Not three.
- **No mixing cosmetic + security** — rename / reformat / lint-style
  cleanups go in their own commit, separately, NEVER bundled with a
  security change. Per user constraint (prompt §7).
- **No `--allow-empty`** — if there is nothing to commit, do not
  commit.
- **No amending merged commits** — fix forward with a new commit.

### 6.3 Push cadence

After every major chunk (= every completed sub-spec, or every 4-5
commits whichever sooner). Per user constraint (prompt §5.5). The pre-push
hook runs the regression sweep (§7).

### 6.4 Forbidden in commits

- Any `tracing::` macro with `?token`, `?api_key`, `?secret`,
  `?password`, `?bearer`, `?auth`, `?url` (where the named variable
  contains the secret). Caught by `TM-LEAK.1` grep before push.
- API keys or session tokens in any file under git tracking. Caught by
  gitleaks pre-commit (`TM-SECRET.8`).
- New `unsafe` blocks. CI fails on any new `unsafe` (`TM-CODE.2`).
- New unbounded channels in hot paths (`TM-CODE.5`).
- New `unwrap()` / `expect()` / `panic!()` outside tests (`TM-CODE.3`).

### 6.5 Pre-commit hook scope

Installed by sub-spec 4.7. Runs (≤ 5 s total):

1. `cargo fmt --check` (formatting).
2. `cargo clippy --workspace --all-targets -- -D warnings` (lints).
3. `gitleaks detect --no-banner` (secrets).
4. `scripts/security_invariants.sh` (tracing-leak grep + derived-Debug
   heuristic).

Pre-commit failures block the commit. No `--no-verify` (§5.6).

---

## 7. Pre-step regression sweep checklist

Before **every** sub-spec PR (i.e. before each "major chunk push" in
§6.3), the executor MUST run this checklist. It exists because Round 1-3
mitigations have been the foundation of three months of work; losing one
silently is worse than missing a new mitigation.

### 7.1 What 2026-05-23 sweep confirmed GREEN

| # | Check                                                                                    | Verified via                                                                                                |
|---|------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------|
| 1 | `define_secret_token!` macro intact (Zeroizing, ct_eq, manual Debug "(***)", validating Serde) | `crates/claude-phone-shared/src/token.rs:38-169`                                                            |
| 2 | CSP `default-src 'self'`, `script-src 'self'`, `frame-ancestors 'none'`                  | `crates/claude-phone-gateway/src/http.rs:91-106`                                                            |
| 3 | HSTS `max-age=63072000; includeSubDomains; preload`                                       | `crates/claude-phone-gateway/src/http.rs:113-116`                                                           |
| 4 | Permissions-Policy `geolocation=(), microphone=(), camera=()`                            | `crates/claude-phone-gateway/src/http.rs:129-132`                                                           |
| 5 | X-Frame-Options DENY, X-Content-Type-Options nosniff, Referrer-Policy no-referrer        | `crates/claude-phone-gateway/src/http.rs:117-128`                                                           |
| 6 | Server header hidden as `claude-phone`                                                    | `crates/claude-phone-gateway/src/http.rs:135-138`                                                           |
| 7 | Origin enforcement on `/api/wrapper`                                                      | `crates/claude-phone-gateway/src/routes/wrapper_ws.rs:40-46`                                                |
| 8 | Origin enforcement on `/api/phone/:token`                                                 | `crates/claude-phone-gateway/src/routes/phone_ws.rs:45-51` (note: passes through on missing Origin — P0-7)  |
| 9 | Slow-loris recv_hello timeout 10 s (wrapper_ws)                                           | `crates/claude-phone-gateway/src/routes/wrapper_ws.rs:33,191-194`                                           |
| 10 | `MAX_WS_MESSAGE_BYTES = 64 KB` on both routes                                            | wrapper_ws.rs:27, phone_ws.rs (similar)                                                                     |
| 11 | redact_path in TraceLayer for `/s/<token>` and `/api/phone/<token>`                       | `crates/claude-phone-gateway/src/http.rs:19-29,78-82`                                                       |
| 12 | gateway-dev.toml placeholder invalid length (rejected at config load)                     | `gateway-dev.toml:12`                                                                                       |
| 13 | Wrapper RPC bearer auth on `/pair` and `/status`                                         | `crates/claude-phone-wrapper/src/rpc.rs:108-128`                                                            |
| 14 | `main.rs has_public_url: bool` (no `?public_url`)                                        | `crates/claude-phone-wrapper/src/main.rs:142-144`                                                           |
| 15 | `PairResponse` manual Debug redacted                                                      | `crates/claude-phone-wrapper/src/rpc.rs:32-40`                                                              |
| 16 | cargo-audit + npm-audit + Dependabot in CI                                                | `.github/workflows/ci.yml`, `.github/dependabot.yml`                                                        |

### 7.2 Sweep procedure (run before every sub-spec push)

```bash
# 1. Grep that none of the above sentinels changed signature
grep -nE 'define_secret_token!' crates/claude-phone-shared/src/token.rs
grep -nE "content-security-policy|strict-transport-security|x-frame-options" \
    crates/claude-phone-gateway/src/http.rs
grep -nE "(Origin|ORIGIN).*public_origin" \
    crates/claude-phone-gateway/src/routes/{wrapper_ws,phone_ws}.rs

# 2. Tests pass
cargo test --workspace
npm -w web test

# 3. Lints clean
cargo clippy --workspace --all-targets -- -D warnings

# 4. Format clean
cargo fmt --check

# 5. Audit clean
cargo audit
( cd web && npm audit --omit dev )

# 6. Forward-looking invariants
bash scripts/security_invariants.sh
```

If any step fails, the sub-spec PR is blocked until the regression is
restored. **Never** "fix" a regression by deleting the failing test
(prompt §7). Root-cause and restore the guard.

### 7.3 Known PARTIAL state (single carry-over)

Phone-side `/api/phone/:token`:

- Origin check passes through when `Origin` header is **missing**.
  Browsers always send it; non-browser clients may not. **Fix in
  sub-spec 4.13** as `TM-WS.3` (fail-closed when `public_origin` is
  set + Origin missing).
- HTTP upgrade-phase timeout currently relies on hyper default (not
  explicit). **Fix in sub-spec 4.13** as `TM-WS.10`.

These are flagged for P0-7, not regressions of Round 1-3.

---

## 8. TM coverage matrix

Every `TM-CAT.N` from threat model §13 maps to either a sub-spec / P2
mini-section here, or is **Accepted risk (deferred)**. CI script
`TM-LEAK.4` (in sub-spec 4.15) walks both directions: catalog → code,
code → catalog.

### 8.1 Catalog → sub-spec

(see §3 sub-spec index for the full table; this section is the inverse
listing of "where does this TM-CAT.N land")

| TM-CAT.N range  | Status               | Owner                                                          |
|-----------------|----------------------|----------------------------------------------------------------|
| TM-AUTH.*       | mix GREEN / TODO     | Sub-spec 4.2 (auth); TM-AUTH.1/2/5/8/9/10/11 already GREEN     |
| TM-TLS.*        | mix GREEN / TODO     | Sub-spec 4.3 (transport); TM-TLS.2/5/9 already GREEN           |
| TM-INPUT.*      | all TODO             | Sub-spec 4.5 (input)                                            |
| TM-RATE.*       | mix GREEN / TODO     | Sub-spec 4.6 (rate); TM-RATE.8 already GREEN                   |
| TM-SECRET.*     | mix GREEN / TODO     | Sub-spec 4.7 (secrets); TM-SECRET.2/3/4/5/6/7/9/12 already GREEN |
| TM-SUPPLY.*     | mix GREEN / TODO     | §4.8 inline (cargo-deny); rest already GREEN                   |
| TM-INFRA.*      | all TODO             | Sub-spec 4.9 (infra)                                            |
| TM-FRONT.*      | mix GREEN / DEFER    | §4.12 inline (FRONT.3, .4, .11); .1/.2/.12/.13 DEFER (P2)      |
| TM-WS.*         | mix GREEN / TODO     | Sub-spec 4.13 (ws); WS.1/2/4/5/6/11 already GREEN              |
| TM-CODE.*       | mix GREEN / TODO     | Sub-spec 4.14 (code); CODE.7 already GREEN                     |
| TM-TEST.*       | mostly TODO          | Sub-spec 4.15 (testing)                                         |
| TM-LEAK.*       | all TODO             | Sub-spec 4.15 (testing); enforced in CI                        |

### 8.2 Sub-spec → catalog (compact form)

Listed in §3 table. Sub-specs will reproduce the relevant rows verbatim
in their own §2 ("scope") so each is self-readable.

### 8.3 Coverage matrix enforcement (`TM-LEAK.4`)

Sub-spec 4.15 introduces `scripts/tm_coverage.sh`:

1. Parse threat model §13 → set of all `TM-CAT.N` IDs.
2. `grep -rE 'TM-[A-Z]+\.[0-9]+' crates/ web/ deploy/ scripts/ docs/` →
   set of all IDs referenced.
3. Diff. Catalog IDs not referenced anywhere → fail (unless threat model
   §13 marks them `DEFER (P2)` or threat model §9 lists them
   "Accepted risk").
4. Referenced IDs not in catalog → fail (typo or stale reference).

CI runs this on every push.

---

## 9. Open-question resolution

Threat model §11 lists 9 open questions with defaults. This master spec
**adopts all defaults verbatim** unless the user flags otherwise during
Stage 1 review. Restated here for visibility:

| ID   | Question                                                       | Default applied                                                                                  | Owned by sub-spec   |
|------|----------------------------------------------------------------|--------------------------------------------------------------------------------------------------|----------------------|
| OQ-1 | TLS 1.3 only vs 1.2 fallback?                                  | TLS 1.3 only (Caddy `protocols tls1.3`)                                                          | 4.3 transport        |
| OQ-2 | OCSP stapling on / off?                                        | On (Caddy default)                                                                               | 4.3 transport        |
| OQ-3 | HSTS preload registry submission timing?                       | After 30 d of clean production (manual ops in runbook)                                            | §4.10 mini-spec      |
| OQ-4 | `tower-governor` algorithm — fixed window vs sliding?          | Sliding (`governor` default, leaky bucket)                                                       | 4.6 rate-limit       |
| OQ-5 | fail2ban jail list                                             | sshd + recidive + custom claude-phone (watches gateway auth-failure log)                          | 4.9 infra            |
| OQ-6 | auditd watch list                                              | `/etc/claude-phone/`, `/opt/claude-phone/`, `/etc/systemd/system/claude-phone-gateway.service`   | 4.9 infra            |
| OQ-7 | Wrapper config file perms target                               | 0600 (user-only, no group)                                                                       | 4.7 secrets          |
| OQ-8 | Pre-commit hook tool                                           | gitleaks (single-binary, simple setup)                                                            | 4.7 secrets          |
| OQ-9 | OSC/DCS filter location                                        | Gateway phone→wrapper path (defense in depth; xterm.js handles phone side anyway)                | 4.5 input            |

---

## 10. Quality bar (recap from prompt §6)

These are pass/fail criteria for each sub-spec and each commit. None is
soft.

- **No placeholders.** `TBD`, `TODO`, "fill in later" anywhere in a sub-spec
  or in code = plan failure. The only allowed "TODO" markers are in
  threat model §13 status column (where they mean "pending sub-spec") and
  in OQ default tables.
- **Forward-looking tests for every fix.** §5.2 + §5.3.
- **No new deps without justification.** Each new crate (e.g.
  `tower-governor`, `tower-default-headers`) or npm package must justify
  in sub-spec why an in-house solution is worse. Justification in sub-spec
  §3 ("dependencies introduced").
- **No bloat.** No features outside threat model scope. If during execution
  a new mitigation is discovered, it lands in threat model §13 first (new
  `TM-CAT.N`), then in code.
- **Minimum diff per commit.** No drive-by refactors. Cosmetic cleanups
  separately (§6.2).
- **WHY comments only.** Per global CLAUDE.md. No `// increments x by 1`.
  All security comments are `// TM-CAT.N: <reason>` style.
- **All tests green before push.** §5.5.
- **clippy `-D warnings` clean.** §5.4.
- **No commit with secrets.** §6.4 (gitleaks pre-commit).

---

## 11. Stage / approval gates

This hardening pass is staged. Each stage requires explicit user approval
before proceeding to the next.

### Stage 1 — Threat model + master spec (current stage)

Deliverables (this commit):

- `docs/superpowers/specs/2026-05-23-threat-model.md` (written 2026-05-23,
  approved 2026-05-23 contingent).
- `docs/superpowers/specs/2026-05-23-security-hardening-design.md` (this
  file).

User gate: "approve" or "request changes". On approval → Stage 2.

### Stage 2 — Sub-specs (9 documents)

Deliverables:

- `2026-05-23-sec-4.2-auth.md`
- `2026-05-23-sec-4.3-transport.md`
- `2026-05-23-sec-4.5-input.md`
- `2026-05-23-sec-4.6-rate-limit.md`
- `2026-05-23-sec-4.7-secrets.md`
- `2026-05-23-sec-4.9-infra.md`
- `2026-05-23-sec-4.13-websocket.md`
- `2026-05-23-sec-4.14-code-audit.md`
- `2026-05-23-sec-4.15-testing.md`

Each sub-spec follows the template:

1. **Scope** — explicit list of `TM-CAT.N` covered.
2. **Dependencies introduced** — new crates / npm packages, each justified.
3. **Detailed changes per file** — function-level granularity.
4. **Test plan** — behavioural + forward-looking + CI grep, in 3 columns.
5. **Commit boundaries** — one commit per task; ordered list.
6. **Risk-of-regression notes** — what existing test must keep passing.
7. **Pre-step regression sweep checklist reference** (§7 of this file).

User gate per sub-spec **or** batch (user decides). Default: batch
review (user reviews all 9 together; allows cross-cutting feedback).

### Stage 3 — Plan (via writing-plans skill)

Deliverable: `docs/superpowers/plans/2026-05-23-security-hardening-plan.md`
generated by `superpowers:writing-plans`. Bite-sized tasks per category;
TDD ordering; commit-by-commit listing.

User gate: "approve plan". On approval → Stage 4.

### Stage 4 — Execution (via executing-plans skill)

Per prompt §5: `superpowers:executing-plans`, frequent commits, push to
`origin/main` after each major chunk. Regression sweep (§7) before each
push.

### Stage 5 — Validation

After all P0 + P1 sub-specs land:

- `cargo test --workspace` green.
- `npm -w web test` green.
- `cargo clippy --all-targets -- -D warnings -W clippy::pedantic` clean
  (pedantic mode for final pass).
- `cargo audit` clean.
- `cargo deny check` clean.
- `bash scripts/security_invariants.sh` clean.
- `bash scripts/tm_coverage.sh` clean.
- `testssl.sh claude-phone.pl` — all A grades.
- Manual: hstspreload.org eligibility check.
- Manual: fail2ban + auditd + ufw status verified on home server.

Failure of any item → stage 4 not done; remediate before declaring
complete.

---

## 12. Change log

- 2026-05-23 — Initial draft. Companion to `2026-05-23-threat-model.md`.
  Pending Stage 1 approval.

---

## 13. References

- `2026-05-23-threat-model.md` — authoritative threat list, asset
  ranking, `TM-CAT.N` catalog.
- `2026-05-22-test-coverage-and-leakage-prevention-design.md` — sister
  spec covering M9.4 round 1 hardening items, template tone reference.
- `docs/security.md` — legacy v1, to be reduced to a pointer to this
  master + companion threat model after Stage 1 approval.
- Prompt source: `C:\Users\mrzyg\Documents\prompt-claude.txt` (15
  in-scope categories §4, methodology §5, quality bar §6, constraints
  §7-§8).
- Round 1-3 commits: `be60102`, `7d1fd1d`, `6e5c63d`.
