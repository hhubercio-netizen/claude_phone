# Threat Model — Claude Phone (Server Hardening)

**Status:** Draft (brainstorming session 2026-05-23) — pending user approval
**Owner:** main branch, direct commits to `origin/main`
**Implementation tracker:** see `2026-05-23-security-hardening-design.md` (master spec) and per-category sub-specs `2026-05-23-sec-4.X-*.md`
**Supersedes:** `docs/security.md` v1 (T1-T10 table) — to be replaced by pointer + summary after approval

---

## 1. Context and motivation

Claude Phone is a bridge between a Claude Code session running on a developer's
machine and a phone via PWA. The architecture has multiple trust boundaries
(wrapper, gateway, phone, infra) and one shared origin (`claude-phone.pl`
behind Cloudflare). After three security hardening rounds (commits `be60102`,
`7d1fd1d`, `6e5c63d`) addressing 22 concrete leakage / CSWSH / slow-loris
vectors, the project meets baseline production hygiene but lacks a formal
threat model.

This document is the authoritative source of truth for:

- What assets are in scope (§3) and how they are ranked
- Who the threat actors are and what they can do (§4)
- What STRIDE category applies where in the system (§5)
- What attack trees exist for the top five assets (§6)
- What systematic gaps remain (leakage taxonomy §7) and how we prevent regression
- What is **explicitly out of scope** (§9) so the spec does not sprawl
- What sub-specs cover which mitigations and at what priority (§10)

Every concrete mitigation in the master spec and its sub-specs references a
`TM-CAT.N` identifier from this document. Conversely, every `TM-CAT.N` must
have either a corresponding mitigation **or** an explicit "accepted risk" entry.

---

## 2. System and trust boundaries

### 2.1 Component overview

```
+---------------------+                              +---------------------+
|  Dev machine        |     https/wss               |  Phone (browser)    |
|  (wrapper + claude) |  +---------------+          |                     |
|                     |  |               |          |                     |
|  - claude-phone     |--+  Cloudflare   +----------+                     |
|    -wrapper         |  |  (CDN + TLS)  |          |                     |
|  - PTY child:       |  |               |          |                     |
|      claude CLI     |  +-------+-------+          +---------------------+
|  - local RPC bound  |          | wss
|    on 127.0.0.1     |          | (Cloudflare Tunnel)
|                     |          |
+---------------------+          v
                           +-----+-----+
                           |  Caddy    |   reverse proxy, TLS Full strict
                           +-----+-----+
                                 | http (127.0.0.1:8080, loopback only)
                                 v
                           +-----+-----+
                           |  Gateway  |   axum, single binary
                           |  (axum)   |
                           +-----------+
                           home Ubuntu 22.04, single-tenant
```

### 2.2 Trust boundaries (T-N)

| ID  | Boundary                            | Transport                  | Auth                                                                  |
|-----|-------------------------------------|----------------------------|-----------------------------------------------------------------------|
| T1  | Wrapper ↔ Gateway                   | wss over CF Tunnel + Caddy | API key bearer (`WrapperHello.api_key`), ct-eq vs allowlist           |
| T2  | Phone ↔ Gateway                     | wss browser → CF → Caddy   | Session token in URL path (capability URL), validated by registry    |
| T3  | Wrapper ↔ Claude (PTY child)        | PTY (fd-based)             | Same trust domain                                                     |
| T4  | Plugin / pair helper ↔ Wrapper RPC  | HTTP loopback 127.0.0.1    | Bearer in `Authorization: Bearer <ephemeral_api_key>` (env-propagated)|
| T5  | Caddy ↔ Gateway                     | HTTP loopback              | None (single-host trust domain)                                       |
| T6  | Cloudflare ↔ Caddy                  | Cloudflare Tunnel (mTLS)   | CF-managed (`cloudflared`)                                            |
| T7  | Browser ↔ Cloudflare                | https (CF cert)            | None (public surface; access gated downstream)                        |

### 2.3 Data flows and sensitivity

| Flow                | Source                            | Sink                                       | Sensitivity                                                        |
|---------------------|-----------------------------------|--------------------------------------------|--------------------------------------------------------------------|
| PTY output          | Claude CLI in PTY                 | Wrapper → Gateway → Phone                  | **High** — code, AI content, possibly typed secrets                |
| PTY input           | Phone keystrokes                  | Gateway → Wrapper → PTY                    | **High** — may include passwords/tokens typed in-band              |
| Session token       | Gateway (`SessionToken::generate`)| Wrapper → /pair → phone (QR + URL)         | **Critical** — bearer-equivalent for PTY mirror                    |
| API key             | `/etc/claude-phone/gateway.toml`  | Wrapper config → WrapperHello              | **Critical** — long-lived auth credential                          |
| Wrapper RPC bearer  | Wrapper at startup                | env var of direct child process            | **Sensitive** — ephemeral, local-only, mints session tokens        |
| Origin URL          | Gateway config                    | WS handshake / pages                       | Low — public                                                       |

---

## 3. Asset inventory and ranking

Per user decision (brainstorming 2026-05-23):

| Rank | Asset                                       | Why it matters                                                                                                | What compromise enables                                                                                       |
|------|---------------------------------------------|----------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------|
| 1    | **Session tokens** (`SessionToken`)         | Bearer-equivalent for PTY mirror; 256 bit; capability URL exposed in `qr_ascii` and `/s/<token>` page         | Read + write the live PTY mirror, see all session content, type keystrokes (PTY = effective shell access)     |
| 2    | **API keys** (`ApiKey`)                     | Long-lived (90 d rotation recommended), in gateway config; lets wrappers register sessions                    | Register fake wrapper sessions, then phish/brute session tokens, then mount mirror                            |
| 3    | **Wrapper-host RCE via PTY mirror**         | Phone is write-side input to dev-machine shell. Shell escapes via xterm OSC/DCS or typed commands             | Game over — attacker gets dev-machine shell                                                                   |
| 4    | **Session content exfiltration**            | PTY stream carries code, dev-machine paths, AI session content (potentially company code, customer data, in-band secrets) | Information disclosure, downstream credential theft                                                          |
| 5    | **Phone session hijack (network)**          | Wi-Fi MITM / evil twin / BGP hijack captures session-token URL                                                | Equivalent of #1 but requires network position                                                                |

### 3.1 Out of asset scope

- **Anthropic Claude API key** — typed/configured into `claude` CLI itself. Lives in user home dir on dev-machine. Compromise is a Claude-side concern. We do not log/persist/transport it through our pipe; we only PTY-mirror what `claude` displays. *Mitigation responsibility: user.*
- **User's home directory contents** (`~/.ssh`, `~/.aws`, etc.) — wrapper does not read them. RCE on wrapper-host (#3) would expose them but they are not assets we own.
- **Other tenants on the home server** — single-tenant deployment.

---

## 4. Threat actor profiles

All four actors are in scope per user decision. Defense-in-depth across all
profiles; some mitigations cover multiple actors.

### A1. Motivated targeted attacker

- **Motivation**: specific interest in the user — competitor, hostile state, someone targeting Anthropic developer accounts for upstream supply-chain reach.
- **Capabilities**: weeks of recon budget; reads axum/tokio source on the (now-private) repo if leaked; can fuzz the public surface; can register CF lookalike domains for phishing; can MITM Wi-Fi the user briefly uses.
- **TTPs**: targeted spear-phishing with crafted pair-URL lookalikes; long-running brute force on `/api/wrapper`; reading client-side JS for endpoints; CT-log monitoring for sibling-domain misissuance.
- **Mitigations focused on**: `TM-AUTH.*`, `TM-TLS.*` (esp. `TM-TLS.4` CT monitoring), `TM-RATE.*`, `TM-SECRET.*`.

### A2. Script kiddie + automated bots (mass scan)

- **Motivation**: opportunistic; automated tooling crawling for low-hanging fruit.
- **Capabilities**: high volume / low sophistication; botnets, Censys/Shodan scanners; known-CVE PoC payloads; pre-built CF/CDN bypass attempts.
- **TTPs**: path-scan `/.git`, `/.env`, `/wp-admin`, `/phpmyadmin`; method abuse (TRACE, OPTIONS, CONNECT); header-fingerprint scanning; slowloris DoS on small personal sites; npm/cargo malware injection if Dependabot is loose.
- **Mitigations focused on**: `TM-SECRET.4` (hidden server banner), `TM-INPUT.*` (404 hygiene), `TM-RATE.*`, `TM-INFRA.*` (firewall + fail2ban), CF free-tier WAF rules.

### A3. Wi-Fi MITM / network attacker

- **Motivation**: opportunistic capture of credentials and sessions from people on insecure networks.
- **Capabilities**: active MITM on local Wi-Fi (ARP spoofing, evil twin, rogue DNS); HTTPS strip / downgrade; BGP hijack (rare, expensive); sniffing public Wi-Fi.
- **TTPs**: SSL strip on any plaintext-redirect endpoint; cookie theft via plain-HTTP exposure; certificate substitution if HSTS not enforced (first-visit risk); replay of captured authenticated requests.
- **Mitigations focused on**: `TM-TLS.*` (TLS 1.3, HSTS preload, CT monitoring), `TM-WS.*` (wss only).

### A4. Malicious phone owner + insider risk

- **Motivation**: someone with a *legitimate* api_key (former user, disgruntled friend, ex-colleague) abuses the system.
- **Capabilities**: full protocol knowledge; valid api_key; multiple devices for concurrent sessions; can craft OSC/DCS in PTY input.
- **TTPs**: replay own token across devices; attempt to register a token belonging to another user; exfil via OSC 52 clipboard write; hyperlink injection via OSC 8; memory exhaustion via fast I/O.
- **Mitigations focused on**: `TM-AUTH.3` (single-phone), `TM-AUTH.4` (revocation), `TM-INPUT.1-3` (PTY filtering), `TM-RATE.4` (memory cap, 64 KiB), `TM-AUTH.7` (auth-event audit log).

---

## 5. STRIDE per component

Each cell answers: which STRIDE category applies, at what severity, and which
`TM-CAT.N` mitigation(s) address it. "GREEN" marks already-in-place; bare
`TM-CAT.N` marks planned in sub-spec.

### 5.1 Wrapper (claude-phone-wrapper)

| STRIDE | Threat                                                                      | Severity | Mitigation                                                                                            |
|--------|-----------------------------------------------------------------------------|----------|-------------------------------------------------------------------------------------------------------|
| S      | Local attacker on dev-machine impersonates wrapper RPC client (browser at 127.0.0.1, rogue VS Code extension) | High     | `TM-AUTH.8` RPC bearer auth (rpc.rs:108-128) **GREEN**                                                |
| T      | Local attacker overwrites `gateway-dev.toml` to inject malicious api_key    | Medium   | `TM-SECRET.10` wrapper config file perms 0600 (sub-spec 4.7)                                          |
| R      | Wrapper denies that it initiated a session                                  | Low      | Single-user dev environment; audit log informational only                                             |
| I      | wrapper.log leaks session token or api_key                                  | High     | `TM-SECRET.3` manual Debug (**GREEN**), `TM-SECRET.7` main.rs has_public_url (**GREEN**), `TM-SECRET.5` redacted PairResponse Debug (**GREEN**) |
| D      | Local DoS on wrapper RPC                                                    | Low      | 127.0.0.1 only + bearer auth (**GREEN**)                                                              |
| E      | RPC bearer exfil → attacker mints session tokens                            | Critical | `TM-AUTH.8` bearer ephemeral per startup (**GREEN**); env-propagated to direct child only             |

### 5.2 Gateway (claude-phone-gateway)

| STRIDE | Threat                                                                          | Severity | Mitigation                                                                                          |
|--------|---------------------------------------------------------------------------------|----------|-----------------------------------------------------------------------------------------------------|
| S      | Attacker forges WrapperHello with guessed api_key                               | High     | `TM-AUTH.1` 256-bit entropy + `TM-AUTH.2` ct-eq (**GREEN**), `TM-RATE.2` auth-attempt rate limit    |
| S      | Attacker forges Origin header to bypass CSWSH check                             | Medium   | `TM-WS.1` wrapper_ws Origin (**GREEN**), `TM-WS.2` phone_ws Origin (**GREEN**), `TM-WS.3` missing-Origin fail-closed when public_origin configured |
| T      | Attacker tampers with frames in flight                                          | Critical | `TM-TLS.1` TLS 1.3 only, `TM-TLS.2` HSTS preload                                                    |
| R      | Auth attempt not logged → no audit trail                                        | Medium   | `TM-AUTH.7` structured auth-failure log with correlation ID                                          |
| I      | Token in URL leaks via Referer or tracing log                                   | High     | `TM-TLS.5` Referrer-Policy no-referrer (**GREEN**), `TM-SECRET.6` redact_path in TraceLayer (**GREEN**) |
| I      | 500 response includes stack trace                                               | Medium   | `TM-SECRET.11` opaque error chain in `error.rs`                                                     |
| D      | DoS via WS connection flood, slow-loris, slow-write, frame-size abuse           | High     | `TM-WS.4/5` size caps (**GREEN**), `TM-RATE.8` slow-loris recv_hello (**GREEN** wrapper-side), `TM-RATE.1` per-IP cap |
| D      | Memory exhaustion via replay buffer growth                                      | High     | `TM-RATE.4` 64 KiB per-session cap (drop oldest on overflow, `session.rs:24`)                        |
| D      | FD exhaustion                                                                   | Medium   | `TM-INFRA.6` systemd LimitNOFILE, kernel ulimit                                                     |
| E      | Token registration race (TOCTOU)                                                | Medium   | `TM-CODE.4` session/registry lock-ordering audit                                                    |

### 5.3 Web frontend (browser)

| STRIDE | Threat                                                                          | Severity | Mitigation                                                                                          |
|--------|---------------------------------------------------------------------------------|----------|-----------------------------------------------------------------------------------------------------|
| S      | Lookalike domain serves crafted JS to steal token from URL                      | High     | `TM-TLS.2` HSTS preload, `TM-TLS.4` CT monitoring                                                   |
| T      | DOM injection via XSS → reads localStorage or session token                     | High     | `TM-FRONT.6` CSP default-src 'self' (**GREEN**), `TM-FRONT.1` nonce-based style (deferred P2), `TM-FRONT.2` Trusted Types (deferred P2) |
| R      | n/a                                                                             | -        | -                                                                                                   |
| I      | Token in URL exposed via browser history, referer, copy-paste                   | High     | `TM-TLS.5` Referrer-Policy (**GREEN**), `TM-FRONT.3` `history.replaceState()` to clean URL          |
| I      | Console.log of raw WS frame leaks token                                         | Medium   | Already mitigated per `2026-05-22-test-coverage-and-leakage-prevention-design.md` §3.4              |
| D      | Service worker abuse to hold stale token cached                                 | Low      | `TM-FRONT.4` SW scope minimal                                                                       |
| E      | SW cache poisoning → persistent XSS                                             | Medium   | `TM-FRONT.4` SW served same-origin only; CSP enforces                                               |

### 5.4 Infrastructure (Cloudflare + Caddy + Ubuntu host)

| STRIDE | Threat                                                                          | Severity | Mitigation                                                                                          |
|--------|---------------------------------------------------------------------------------|----------|-----------------------------------------------------------------------------------------------------|
| S      | SSH brute force on home server                                                  | High     | `TM-INFRA.3` sshd PermitRootLogin no, password auth off, `TM-INFRA.4` fail2ban sshd jail            |
| T      | Attacker writes `/etc/claude-phone/gateway.toml` outside maintenance window     | High     | `TM-SECRET.1` file perms 0640 root:claude-phone, `TM-INFRA.1` systemd ProtectSystem=strict, `TM-INFRA.5` auditd watch |
| R      | systemd journal lost → no audit trail                                           | Medium   | `TM-INFRA.9` journal persistence; off-host shipping deferred P2                                     |
| I      | Gateway data exfil via process memory dump                                      | Medium   | `TM-INFRA.1` MemoryDenyWriteExecute, `TM-INFRA.8` LimitCORE=0                                       |
| D      | DDoS exhausts CF free-tier limit, gets through to origin                        | Medium   | `TM-RATE.1` origin tower-governor, `TM-INFRA.6` MemoryMax/ulimit cgroup, `TM-INFRA.4` fail2ban      |
| E      | Privilege escalation via service compromise                                     | Critical | `TM-INFRA.1` full systemd hardening (NoNewPrivileges, RestrictAddressFamilies, SystemCallFilter, CapabilityBoundingSet=) |

---

## 6. Attack trees (top 5 assets)

Each tree is `Goal → Sub-goals → Leaves (concrete attack)`. Leaves annotated
with mitigation reference or **Accepted risk** (out of scope per §9).

### 6.1 Asset #1: Session token compromise

```
Goal: Attacker obtains a live session token

├── A. Steal from wire
│   ├── A1. TLS strip on phone → CF leg
│   │   → TM-TLS.2 HSTS preload 2y
│   ├── A2. MITM CF → Caddy (Cloudflare Tunnel terminated by cloudflared)
│   │   → Accepted risk (T6 — trust Cloudflare per §9.2)
│   └── A3. MITM Caddy → Gateway (127.0.0.1)
│       → TM-INFRA.10 single-host kernel-enforced loopback
│
├── B. Steal from origin (logs, memory, disk)
│   ├── B1. tracing log captures token (via WS route span)
│   │   → TM-SECRET.6 redact_path in TraceLayer (GREEN)
│   │   → TM-SECRET.3 manual Debug "***" (GREEN)
│   │   → TM-LEAK.1 CI grep enforcement
│   ├── B2. crash dump / core file contains token in memory
│   │   → TM-INFRA.8 LimitCORE=0
│   │   → TM-SECRET.2 Zeroizing<String> wipes on drop (GREEN)
│   └── B3. journal/logfile readable by other user
│       → TM-SECRET.1 file perms; journal mode=0640
│
├── C. Steal from client (phone)
│   ├── C1. Token in URL → leaks via Referer
│   │   → TM-TLS.5 Referrer-Policy: no-referrer (GREEN)
│   ├── C2. Token cached in browser history
│   │   → TM-FRONT.3 history.replaceState() to clean URL post-load
│   ├── C3. Token persisted to localStorage / sessionStorage
│   │   → TM-FRONT.5 leakage.test.ts enforces (GREEN)
│   ├── C4. XSS exfil
│   │   → TM-FRONT.6 CSP default-src 'self' (GREEN)
│   │   → TM-FRONT.1 nonce-based style-src (P2)
│   └── C5. Malicious browser extension scrapes token from URL
│       → Accepted risk (user-installed extensions are inside trust boundary)
│
├── D. Steal from dev-machine (where token first lives)
│   ├── D1. wrapper.log captures token via tracing
│   │   → TM-SECRET.5 redacted PairResponse Debug (GREEN)
│   │   → TM-SECRET.7 main.rs has_public_url fix (GREEN)
│   ├── D2. local RPC bearer compromise → mint new tokens
│   │   → TM-AUTH.8 bearer ephemeral per startup (GREEN)
│   │   → TM-AUTH.9 env-propagated to direct child only (GREEN)
│   └── D3. plugin/pair helper reads token from RpcState
│       → Accepted risk (helper inside wrapper trust domain)
│
├── E. Brute force token
│   └── E1. Random 43-char base64url strings against /s/<token> or /api/phone/<token>
│       → TM-AUTH.1 256-bit entropy, OsRng (GREEN)
│       → TM-RATE.2 phone-ws rate limit per IP
│
└── F. Replay
    ├── F1. Use captured token after wrapper restart
    │   → TM-AUTH.5 token forgotten on wrapper exit (GREEN — registry drop)
    └── F2. Use captured token concurrently with original phone
        → TM-AUTH.3 single-phone-per-session, REFUSES second while first attached (`SessionTaken`) — strictly safer than kick-previous (which would let an attacker with the token kick out the legit user). Behaviour already in `registry.rs:78-84`; sub-spec 4.2 adds forward-looking test + `// TM-AUTH.3:` comment.
```

### 6.2 Asset #2: API key compromise

```
Goal: Attacker registers a fake wrapper to mint session tokens

├── A. Steal from config file
│   ├── A1. Read /etc/claude-phone/gateway.toml directly
│   │   → TM-SECRET.1 file perms 0640 root:claude-phone
│   ├── A2. Read process memory
│   │   → TM-SECRET.2 Zeroizing<String> wipes on drop (GREEN)
│   │   → TM-INFRA.1 MemoryDenyWriteExecute, no PTRACE
│   └── A3. Read swap / hibernation
│       → Accepted risk (server should not swap; sub-spec 4.9 verifies)
│
├── B. Steal from wire
│   ├── B1. WrapperHello on wrapper → gateway leg captured
│   │   → TM-TLS.1 wss end-to-end (GREEN with caveat T6)
│   └── B2. CF cert misissuance
│       → TM-TLS.4 CT monitoring (crt.sh alerts, sub-spec 4.3)
│
├── C. Steal from dev-machine config
│   ├── C1. User's ~/.config/claude-phone/config.toml mode 0600 not enforced
│   │   → TM-SECRET.10 wrapper config-load enforces perms (sub-spec 4.7)
│   └── C2. Accidentally committed to git
│       → TM-SECRET.9 *-dev.toml gitignored (GREEN)
│       → TM-SECRET.8 gitleaks pre-commit hook (sub-spec 4.7)
│
├── D. Brute force
│   ├── D1. Random 43-char attempts on /api/wrapper
│   │   → TM-AUTH.1 256-bit entropy (GREEN)
│   │   → TM-RATE.2 auth-attempt rate limit (10/IP/min, exp backoff)
│   │   → TM-INFRA.4 fail2ban claude-phone jail on persistent failures
│   └── D2. Credential stuffing from other breaches
│       → TM-AUTH.6 90 d key rotation
│
└── E. Replay
    └── E1. Captured api_key reused after compromise suspected
        → TM-AUTH.4 revocation via revoked.toml + SIGHUP (D2 default)
```

### 6.3 Asset #3: Wrapper-host RCE

```
Goal: Attacker obtains shell on dev-machine via phone-side input

├── A. Direct command injection via PTY input
│   ├── A1. Phone sends raw shell commands as keystrokes
│   │   → Accepted risk (the phone IS the user; this is the feature)
│   │   → Defense: prevent compromise of the phone session (§6.1)
│   └── A2. Crafted control bytes pivot to host commands via shell escapes
│       → TM-INPUT.7 control char sanitization in gateway phone→wrapper path
│
├── B. xterm / terminal sequence abuse
│   ├── B1. OSC 52 clipboard write → poisons user clipboard
│   │   → TM-INPUT.1 strip OSC 52 in gateway phone→wrapper path (D3 default)
│   ├── B2. OSC 8 hyperlinks → phishing future terminal readers
│   │   → TM-INPUT.2 strip OSC 8 (D3 default)
│   ├── B3. OSC 0/1/2 window title set
│   │   → Accepted risk (low impact; window title only)
│   ├── B4. APC / PM / SOS / DCS arbitrary in phone→wrapper direction
│   │   → TM-INPUT.3 reject DCS/APC/PM/SOS in phone→wrapper (claude→phone is allowed)
│   └── B5. Terminal injection via DECRQSS / DECSCUSR
│       → TM-INPUT.4 audit handled CSI/DCS in xterm.js (sub-spec 4.5)
│
├── C. Bypass via WebSocket frame
│   ├── C1. Oversized text frame to bypass binary path
│   │   → TM-WS.4 64 KB max_message_size (GREEN)
│   │   → TM-WS.5 64 KB max_frame_size (GREEN)
│   └── C2. JSON depth bomb in control frame
│       → TM-INPUT.5 serde_json::Deserializer::set_max_recursion
│
├── D. Exploit dep crate (tokio, axum, portable-pty, crossterm)
│   ├── D1. RCE in PTY-handling crate
│   │   → TM-SUPPLY.1 cargo-audit + cargo-deny + Dependabot weekly (mostly GREEN, cargo-deny pending)
│   │   → TM-SUPPLY.3 reproducible builds (sub-spec 4.8, deferred-minimal)
│   └── D2. Malicious update to direct or transitive dep
│       → TM-SUPPLY.2 lockfile pinning (GREEN), Dependabot review process
│
└── E. Wrapper-host already compromised
    → Accepted risk (out of scope per §9.10 — compromised wrapper = game over)
```

### 6.4 Asset #4: Session content exfiltration

```
Goal: Attacker reads PTY content (code, AI session, secrets typed in band)

├── A. Become the phone (token compromise)
│   → See §6.1 in full
│
├── B. Capture on wire
│   → TM-TLS.1 wss end-to-end
│
├── C. Replay-buffer scrape on gateway
│   ├── C1. Attacker becomes wrapper via api_key compromise
│   │   → See §6.2
│   └── C2. Memory dump of gateway process
│       → TM-INFRA.8 LimitCORE=0
│       → TM-INFRA.1 MemoryDenyWriteExecute, no PTRACE
│
├── D. Side-channel
│   ├── D1. Traffic analysis (frame size, timing) reveals typing patterns
│   │   → Accepted risk (padding out of scope at our scale)
│   └── D2. Cache-timing on ct_eq
│       → TM-AUTH.2 subtle::ConstantTimeEq with compiler barrier (GREEN)
│
└── E. Phone-side leak
    ├── E1. xterm.js renders content to canvas → screen-record extension exfil
    │   → Accepted risk (eyes-on data inside trust boundary)
    └── E2. Content written to localStorage / sessionStorage
        → TM-FRONT.5 leakage.test.ts enforces no-storage (GREEN)
```

### 6.5 Asset #5: Phone session hijack (network)

```
Goal: Attacker on local Wi-Fi captures session URL and mounts mirror

├── A. Intercept QR-scanned URL
│   ├── A1. Wi-Fi sniffer captures https to claude-phone.pl/s/<token>
│   │   → TM-TLS.1 TLS 1.3 only, modern cipher suites
│   ├── A2. SSL strip downgrade attempt
│   │   → TM-TLS.2 HSTS preload (first-visit protection)
│   └── A3. Evil twin Wi-Fi with fake cert
│       → TM-TLS.2 HSTS prevents fallback to plain HTTP
│       → Browser-enforced CA trust (PKI baseline)
│
├── B. ARP spoof + cert substitution
│   → TM-TLS.2 HSTS preload
│   → TM-TLS.4 CT monitoring catches misissuance
│
├── C. BGP hijack
│   → TM-TLS.1 + TM-TLS.4 (residual defense)
│   → Accepted risk for nation-state-level BGP attacks
│
└── D. Token already exposed elsewhere
    → See §6.1
```

---

## 7. Leakage taxonomy

Three recurring patterns produced all 22 known leakage vectors found across
Rounds 1-3. Naming them and enforcing detection at CI prevents future
regression.

### 7.1 Pattern L1 — tracing fields with secret payload

**Symptom:** `tracing::info!(?token, …)` or `tracing::warn!(api_key = %k.as_str(), …)`. The `?` operator pulls `Debug`; `%` pulls `Display`. Either path emits the secret if the type's Debug/Display is not redacted.

**Round 3 example:** `wrapper/src/main.rs` previously emitted `tracing::info!(?public_url, …)`. Since `public_url` contains the session token in its path, this wrote the bearer-equivalent to wrapper.log. Fixed by replacing with `has_public_url: bool` (main.rs:142-144).

**Systematic sweep:** all `tracing::(info|warn|error|debug)!` invocations whose formatted arguments match `?|%` followed by an identifier named `token|api_key|secret|password|hash|bearer|auth|url`.

**CI enforcement (`TM-LEAK.1`, sub-spec 4.7):**
```bash
# scripts/security_invariants.sh — L1 sweep
! grep -REn 'tracing::(info|warn|error|debug)!\s*\([^)]*[?%]\s*(token|api_key|secret|password|hash|bearer|auth|url)' crates/ \
  || { echo "L1: tracing macro with secret-named field"; exit 1; }
```

### 7.2 Pattern L2 — derived Debug on container types

**Symptom:** `#[derive(Debug)]` on a struct/enum containing a `SessionToken`/`ApiKey` field. The derived impl prints all fields. Currently safe because the inner types' Debug returns `"(***)"`. But if a future maintainer adds a `String` field embedding the token, the derived Debug leaks it.

**Round 3 example:** `PairResponse` had `#[derive(Debug)]` and three string fields where `url` embeds the token, `token` is the raw string, `qr_ascii` is the QR encoding (visually different but reversibly equivalent). Fixed by manual Debug returning `"<redacted>"` (rpc.rs:32-40).

**Systematic sweep:** for every `#[derive(...Debug...)]` struct/enum, audit field types; flag any `String`/`Vec<u8>` field whose name matches the secret-name regex.

**Enforcement (`TM-LEAK.2`, sub-spec 4.7):** per-area `leakage_test.rs` asserts `format!("{:?}", x)` does not contain known secret values. Mechanical CI grep is heuristic-only (see sub-spec 4.7 for the script).

### 7.3 Pattern L3 — asymmetric guards across symmetric routes

**Symptom:** two routes that should have the same guard, one has it, the other does not. Common after refactor sequences.

**Round 3 example:** Origin enforcement was added to `phone_ws` for CSWSH defense, but `wrapper_ws` was left without it. Fixed by adding the same Origin guard to wrapper_ws.

**Current gaps (post-sweep, address in sub-spec 4.13):**

- Phone WS Origin check **passes through when Origin header is missing**. Real browsers always send Origin; non-browser clients (curl, custom scripts) may not. Recommend: **fail-closed when `public_origin` is configured** (`TM-WS.3`).
- Wrapper WS post-upgrade hello timeout (10 s) ✓; phone WS has no post-upgrade hello (different protocol). HTTP upgrade phase itself has no explicit timeout in either route (`TM-WS.10`).

**Enforcement (`TM-LEAK.3`, sub-spec 4.13):** manual sweep listed in master spec §5.2 and per sub-spec; CI heuristic in `scripts/security_invariants.sh` enumerates known guard categories and matched routes.

### 7.4 Forward-looking discipline

For each mitigation in §6, the corresponding sub-spec adds **three** layers:

1. A **forward-looking test** that fails if the guard is reverted (invariant test, not behavior test).
2. A `// TM-CAT.N: <one-liner>` comment at the guard site so anyone removing it sees the threat-model annotation.
3. A CI grep step (where mechanical) that catches the pattern's reintroduction.

The combination is intentional redundancy — tests can be deleted, comments
ignored, CI greps silenced, but all three at once is sticky.

---

## 8. Privacy data inventory

The system is multi-user (~10 known users with manually issued API keys).
GitHub repository will be flipped to **private** before deploy. Privacy
footprint is therefore minimal but documented for due diligence.

| Data                  | Source                              | Stored where                                                                      | Retention                                          | Justification        |
|-----------------------|-------------------------------------|-----------------------------------------------------------------------------------|----------------------------------------------------|----------------------|
| Client IP             | CF edge → CF logs → origin tracing  | Cloudflare 24 h (free-tier default); gateway tracing 7 d (D7); Caddy access 7 d   | Configured retention                               | Legitimate interest  |
| User-Agent            | Same                                | Same                                                                              | Same                                               | Same                 |
| Session token (URL)   | Gateway `SessionToken::generate`    | Gateway memory (registry), wrapper memory (SessionState), phone URL bar (until `history.replaceState` clean) | Until session end + 7 d sweeper grace              | Bearer auth          |
| API key               | Manual issuance                     | `/etc/claude-phone/gateway.toml`                                                  | Until rotation (90 d)                              | Long-lived auth      |
| PTY content           | Live stream                         | **NOT PERSISTED** (RAM only, replay buffer hard cap 8 MB)                         | Until phone reconnects or session ends             | Operational          |
| Wrapper RPC bearer    | Ephemeral per wrapper startup       | Wrapper memory + env var of child                                                 | Until wrapper exits                                | Local-only auth      |

### 8.1 Privacy policy artifacts (sub-spec 4.11, deferred-minimal)

- `web/public/privacy.html` — bilingual 1-pager (PL + EN brief): data names, retention, contact email.
- No cookie banner: app sets no cookies of its own. CF may set `__cf_bm` (bot management) — disclose this.
- No analytics, no third-party trackers.

### 8.2 Right to erasure procedure

User identifies themselves with their api_key; admin (you) deletes from
`gateway.toml`, restarts gateway, force-rotates if needed. Gateway tracing is
not keyed by user identity (only IP/UA); per-user log scrub is best-effort
grep. Documented in `docs/runbook.md` (referenced from sub-spec 4.11).

---

## 9. Out-of-scope (explicit)

The following are explicitly out of scope for this hardening pass. Each may
be reconsidered later, but spec/plan does not chase them now.

1. **Physical access to home server** — physical security responsibility.
2. **Cloudflare account compromise** — trust boundary baseline (T6). Mitigation is CF account hygiene (2FA, API-token rotation), not our code.
3. **DNS / registrar compromise** — registrar account hygiene; CT monitoring (`TM-TLS.4`) is the residual catch.
4. **Anthropic Claude API key exfil** — user's responsibility (lives in `claude` CLI config).
5. **Supply chain attack at crates.io / npm registry origin** — `cargo-audit` + Dependabot is our first line; vendoring deferred per user.
6. **`cargo-vet` manual review of transitive deps** — overkill at this scale, deferred per user.
7. **HSTS preload registry submission** — manual ops step in runbook, not enforced by code (`TM-TLS.3`).
8. **External status page / off-host log shipping** — deferred P2; `/healthz` endpoint suffices.
9. **mTLS for wrapper ↔ gateway** — over-engineering at our scale; bearer + Origin + TLS is sufficient.
10. **Compromised wrapper-host** — game over by definition; user's responsibility for own machine security.
11. **Insider with admin shell on home server** — operational mitigation (limited admin set, auditd).
12. **Zero-day in tokio/axum/Caddy/cloudflared** — accept risk; Dependabot weekly + cargo-audit catches known CVEs.
13. **Formal SECURITY.md disclosure policy with PGP, bug bounty** — repo is private; minimal SECURITY.md only.
14. **Trusted Types JS API** — deferred P2; CSP `default-src 'self'` + same-origin SW is sufficient.
15. **Cloudflare Bot Management** — Free tier, not available.
16. **Cloudflare Advanced Rate Limiting** — Free tier, not available.
17. **Reproducible / signed release builds (cosign / sigstore)** — deferred P2; tagged commits + Cargo.lock baseline.
18. **Live external pentest engagement** — deferred; in-house pentest-style e2e tests (`TM-TEST.4`) catch regressions on known patterns.

---

## 10. Priority categorization (P0/P1/P2)

Each prompt category (4.X) is classified P0/P1/P2 based on direct link to
top-3 assets and probability of attack from any of the four actors.

### P0 — Full sub-spec, execute first

| Cat  | Title                                      | Sub-spec file                                  | Direct asset link                |
|------|--------------------------------------------|------------------------------------------------|----------------------------------|
| 4.2  | Authentication & Session Management        | `2026-05-23-sec-4.2-auth.md`                   | #1, #2, #5                       |
| 4.3  | Transport Security                         | `2026-05-23-sec-4.3-transport.md`              | All (wire)                       |
| 4.5  | Input Validation & Output Encoding         | `2026-05-23-sec-4.5-input.md`                  | #3, #4                           |
| 4.6  | Rate Limiting & DoS Resistance             | `2026-05-23-sec-4.6-rate-limit.md`             | DoS; brute force on #1, #2       |
| 4.7  | Secrets Management                         | `2026-05-23-sec-4.7-secrets.md`                | #1, #2                           |
| 4.9  | Infrastructure Hardening                   | `2026-05-23-sec-4.9-infra.md`                  | #3 blast-radius containment      |
| 4.13 | WebSocket-specific                         | `2026-05-23-sec-4.13-websocket.md`             | All in-flight                    |
| 4.14 | Code-Level Audit                           | `2026-05-23-sec-4.14-code-audit.md`            | Regression catch for all         |

### P1 — Full sub-spec, execute after P0

| Cat  | Title                                      | Sub-spec file                                  | Reason for P1                                                 |
|------|--------------------------------------------|------------------------------------------------|---------------------------------------------------------------|
| 4.15 | Testing & Verification                     | `2026-05-23-sec-4.15-testing.md`               | Validates P0 — cargo-fuzz + proptest after P0 lands           |

### P2 — Inline in master spec only (no dedicated sub-spec)

| Cat  | Title                                      | Existing state, minimum action                                                                                                  |
|------|--------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------|
| 4.4  | Browser Security Headers                   | Round 2 covered most (CSP, HSTS, X-Frame, X-Content-Type, Permissions-Policy, Referrer-Policy). Nonce upgrade flagged but P2.   |
| 4.8  | Supply Chain Security                      | cargo-audit + npm-audit + Dependabot in CI ✓; add `cargo-deny` config (small task, inline).                                     |
| 4.10 | Operational Security                       | `/healthz` endpoint exists; no formal alerting/statuspage. Document basic incident-response procedure in runbook.                |
| 4.11 | Privacy & GDPR                             | `privacy.html` 1-pager bilingual; no formal procedure. Right-to-erasure as informal admin steps in runbook.                     |
| 4.12 | Frontend-Specific                          | Round 2 covered SW scope + leakage tests; nonce-based style + Trusted Types flagged but P2.                                     |

---

## 11. Open questions / deferred decisions

Default-or-flag semantics: if user does not flag during master-spec review,
master spec proceeds with these defaults.

| ID   | Question                                                                    | Default                                                                                          | Sub-spec     |
|------|-----------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------|--------------|
| OQ-1 | TLS 1.3 only vs 1.2 fallback?                                               | TLS 1.3 only (Caddy `protocols tls1.3`)                                                          | 4.3          |
| OQ-2 | OCSP stapling on / off?                                                     | On (Caddy default)                                                                               | 4.3          |
| OQ-3 | HSTS preload registry submission timing?                                    | After 30 d of clean production                                                                   | 4.3 (manual) |
| OQ-4 | `tower-governor` algorithm — fixed window vs sliding?                       | Sliding (`governor` default, leaky bucket)                                                       | 4.6          |
| OQ-5 | fail2ban jail list                                                          | sshd + recidive + custom claude-phone (watches gateway auth-failure log)                         | 4.9          |
| OQ-6 | auditd watch list                                                           | `/etc/claude-phone/`, `/opt/claude-phone/`, `/etc/systemd/system/claude-phone-gateway.service`   | 4.9          |
| OQ-7 | Wrapper config file perms target                                            | 0600 (user-only, no group)                                                                       | 4.7          |
| OQ-8 | Pre-commit hook tool                                                        | gitleaks (single-binary, simple setup)                                                           | 4.7          |
| OQ-9 | OSC/DCS filter location                                                     | Gateway phone→wrapper path (defense in depth; xterm.js handles phone side anyway)                | 4.5          |

---

## 12. Existing mitigations summary (baseline confirmed by 2026-05-23 sweep)

| Round | Commit       | Mitigations confirmed in place                                                                                                                                    | TM coverage                  |
|-------|--------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------|
| 1     | `be60102`    | Macro-based newtype tokens (Zeroizing, ct_eq, manual Debug "(***)", validating serde with zeroizing visitor)                                                       | TM-AUTH.*, TM-SECRET.*       |
| 2     | `7d1fd1d`    | CSP, HSTS 2 y preload, Permissions-Policy, X-Frame DENY, X-Content-Type-Options nosniff, Referrer-Policy no-referrer, hidden Server header, wrapper RPC bearer, CSWSH defense on phone_ws, cargo-audit + npm-audit + Dependabot | TM-TLS.*, TM-FRONT.*, TM-SUPPLY.* |
| 3     | `6e5c63d`    | L1 tracing leak fix (`has_public_url`), L2 redacted PairResponse Debug, L3 wrapper_ws Origin enforcement, L4 gateway-dev.toml placeholder, L5 slow-loris recv_hello 10 s | TM-SECRET.*, TM-WS.*         |

These mitigations are foundation. New sub-specs **extend** them — they do not
replace them.

---

## 13. Mitigation catalog (`TM-CAT.N` index)

This is the canonical list of mitigation identifiers. Sub-specs reference
these in code comments (`// TM-CAT.N: <reason>`) and in commit messages.

### Authentication & Session (TM-AUTH)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-AUTH.1  | 256-bit token entropy via OsRng                                                             | GREEN  |
| TM-AUTH.2  | Constant-time equality (`subtle::ConstantTimeEq`) on token/key compare                      | GREEN  |
| TM-AUTH.3  | Single-phone-per-session — second phone refused (`SessionTaken`) while first attached (D1 default; strictly safer than kick-previous which would enable attacker takeover of legit user's connection). | GREEN (`registry.rs::attach_phone` carries the TM-AUTH.3 annotation explaining the refuse-second-over-kick-previous asymmetry; `tests/registry_test.rs::second_phone_attach_while_first_attached_fails` asserts the EXACT `SessionTaken` variant (not just any `Err`) so a future refactor that downgrades to 404 trips it; the new `concurrent_phone_attaches_only_one_wins` race-test joins two parallel attaches and proves exactly one wins and the loser is `SessionTaken` specifically, pinning the mutex-serialization invariant against any future lock-free conversion) |
| TM-AUTH.4  | Revocation via `/etc/claude-phone/revoked.toml` + SIGHUP reload (D2 default)                | TODO   |
| TM-AUTH.5  | Token forgotten on wrapper exit (registry drop)                                             | GREEN  |
| TM-AUTH.6  | 90-day API key rotation cadence (runbook documented)                                        | TODO   |
| TM-AUTH.7  | Structured auth-failure log with correlation ID                                             | TODO   |
| TM-AUTH.8  | Wrapper RPC bearer ephemeral per startup (env-propagated to direct child)                   | GREEN  |
| TM-AUTH.9  | Wrapper RPC bearer not persisted anywhere                                                   | GREEN  |
| TM-AUTH.10 | Session token URL never logged at any tier (path redaction + tracing leak prevention)       | GREEN  |
| TM-AUTH.11 | Sweeper idle-timeout (7 d) confirmed evicts; sweeper interval = min(60s, timeout/4)         | GREEN  |

### Transport / TLS (TM-TLS)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-TLS.1   | TLS 1.3 only at Caddy edge                                                                  | GREEN (deploy/caddy/Caddyfile `protocols tls1.3`) |
| TM-TLS.2   | Strict-Transport-Security `max-age=63072000; includeSubDomains; preload`                    | GREEN (aligned across gateway http.rs and Caddy)  |
| TM-TLS.3   | HSTS preload registry submission (manual ops, after 30 d clean)                             | DEFER  |
| TM-TLS.4   | CT monitoring via crt.sh alerts on `claude-phone.pl`                                        | GREEN (`.github/workflows/ct-monitor.yml` + `scripts/ct_monitor.sh`; daily 06:00 UTC, opens/updates tracking issue on novel serial; baseline cached across runs) |
| TM-TLS.5   | Referrer-Policy `no-referrer`                                                               | GREEN  |
| TM-TLS.6   | OCSP stapling (Caddy default on)                                                            | GREEN (`deploy/scripts/post_deploy_verify.sh` openssl s_client -status; invoked from `deploy.sh` with STRICT=1) |
| TM-TLS.7   | TLS scan (testssl.sh) — all A grades                                                        | GREEN (`deploy/scripts/post_deploy_verify.sh` testssl.sh --jsonfile; fails on HIGH/CRITICAL under STRICT=1) |
| TM-TLS.8   | Cloudflare TLS mode Full (strict)                                                           | GREEN (`deploy/scripts/post_deploy_verify.sh` CF API `/zones/.../settings/ssl` check; STRICT=1 aborts deploy on non-`strict` mode; `deploy/cloudflare/README.md` documents token+zone provisioning) |
| TM-TLS.9   | WS over wss only, never ws (Origin + scheme check)                                          | GREEN  |

### Input validation / Output encoding (TM-INPUT)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-INPUT.1 | Strip OSC 52 (clipboard write) in gateway phone→wrapper direction                            | GREEN (`sanitize_phone_input` in `phone_ws.rs` strips every `ESC ]` OSC sequence including OSC 52; unit tests pin OSC-52 BEL- and ST-terminated cases; asymmetric guard asserts the call site is phone-only) |
| TM-INPUT.2 | Strip OSC 8 (hyperlinks) in gateway phone→wrapper direction                                  | GREEN (same sanitizer drops every OSC, OSC 8 included; CSI / SS3 / bracketed-paste preserved by allow-list) |
| TM-INPUT.3 | Reject DCS/APC/PM/SOS in gateway phone→wrapper direction                                     | GREEN (sanitizer also strips `ESC P` DCS, `ESC _` APC, `ESC ^` PM, `ESC X` SOS; truncated sequences drop the remainder so a half-built OSC cannot leak into the PTY) |
| TM-INPUT.4 | Audit xterm.js CSI/DCS handlers; disable OSC 52, OSC 8 client-side                           | TODO   |
| TM-INPUT.5 | `serde_json::Deserializer::set_max_recursion` on all WS JSON parsing                         | GREEN (`protocol.rs` module doc pins the serde_json default 128-level cap as the wire-contract invariant; forward-looking tests `rejects_deeply_nested_json` + `rejects_deeply_nested_control_message` break if anyone enables `unbounded_depth` or swaps the parser) |
| TM-INPUT.6 | Path traversal: tower-http ServeDir reject `..` (verify via test)                            | GREEN (`http.rs` ServeDir annotated; `tests/path_traversal_test.rs` drives 4 raw-TCP cases covering `..`, `%2e%2e`, double-slash, and a legit-asset sanity check; the fixture plants an "outside-assets" canary so the assertions also prove no leak) |
| TM-INPUT.7 | Control-character sanitization on session token from URL                                     | GREEN (`is_base64url_byte` in `token.rs` annotated; `#[cfg(test)] mod tests` covers NUL/BEL/ESC/DEL/slash/backslash/high-bit on both SessionToken and ApiKey; gateway `tests/token_charset_test.rs` drives raw-TCP `/api/phone/<43-byte-with-control-char>` and asserts no 101/200) |
| TM-INPUT.8 | Wrapper CLI `--claude-bin` arg validation (block path traversal)                             | GREEN (`cli.rs::CliError` + `Cli::validate` rejects empty + any C0/DEL byte; `main.rs` switches to `Cli::parse_validated`; `tests/cli_validation_test.rs` covers empty, NUL, newline, tab, DEL, plus deliberate-accept cases for normal/`..`-relative paths) |

### Rate limiting / DoS (TM-RATE)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-RATE.1  | tower-governor per-IP HTTP cap (5 req/s, burst 10) wired in `http.rs` GovernorLayer + `serve.rs` ConnectInfo injection; covered by `tests/rate_limit.rs::per_ip_governor_returns_429_under_burst` | GREEN  |
| TM-RATE.2  | Auth-attempt rate limit (10 failures/IP/60s → exp backoff `2^n` s, cap 1 h) via `AuthRateLimiter` in `rate_limit.rs`; wired in `wrapper_ws::handler` (locked IPs get 429 before upgrade) and `handle_socket` (failure / success counters); covered by `tests/rate_limit.rs::wrapper_auth_failures_trigger_per_ip_lockout` + unit tests in `rate_limit::tests` | GREEN  |
| TM-RATE.3  | Per-connection msg/s rate via `ConnRateLimiter` (100/s phone→gw using `PHONE_TO_GW_MSG_PER_SEC`, 1000/s wrapper→phone using `GW_TO_PHONE_MSG_PER_SEC`); wired in both `wrapper_ws::outgoing_task` and `phone_ws::outgoing_task`. Flooding cancels the session via `session.cancel.cancel()`. Covered by `tests/rate_limit.rs::wrapper_message_flood_closes_session` + unit tests in `rate_limit::tests`. | GREEN  |
| TM-RATE.4  | Per-session memory cap `PHONE_BUFFER_BYTES_CAP = 64 KiB`, drop oldest on overflow (session.rs:24) | GREEN  |
| TM-RATE.5  | FD exhaustion: systemd LimitNOFILE + warning alert at 80%                                   | GREEN (`deploy/systemd/claude-phone-gateway.service` `LimitNOFILE=8192`; 80% alert deferred per sub-spec 4.9 §1.1, observability owned by future 4.4) |
| TM-RATE.6  | Slow-write defense: bounded mpsc channels (256 frames, `registry.rs`) plus `SINK_SEND_TIMEOUT = 5 s` wrapping every `sink.send` in both `wrapper_ws::incoming_task` and `phone_ws::incoming_task`; timeout / error cancels the session via `session.cancel.cancel()`. Constant-bounds covered by `tests/rate_limit.rs::sink_send_timeout_is_bounded_and_reasonable`; wiring is enforced at compile time via the `SINK_SEND_TIMEOUT` import in both routes (clippy `-D warnings` would catch unused-import regression). | GREEN  |
| TM-RATE.7  | Post-hello idle / no-pong watchdog: every `Message::Pong` stamps `last_pong_ms: Arc<AtomicU64>` (millis since socket open); every 30 s keepalive tick checks `age > PONG_DEADLINE` (90 s) and cancels the session if so. Wired into both `wrapper_ws.rs` and `phone_ws.rs`. Constant-bounds covered by `tests/rate_limit.rs::pong_deadline_is_bounded_and_reasonable`; wiring is enforced at compile time via `PONG_DEADLINE` imports in both routes. | GREEN  |
| TM-RATE.8  | Slow-loris recv_hello timeout 10 s on wrapper_ws AND phone_ws                               | GREEN (`HELLO_TIMEOUT` in `wrapper_ws.rs`, `PHONE_HELLO_TIMEOUT` in `phone_ws.rs`; phone requires a `phone_hello` first frame before any binary bytes reach the PTY; e2e test `phone_ws_requires_phone_hello_before_bridging` pins the gate) |
| TM-RATE.9  | Slow-loris HTTP upgrade timeout: `hyper_util::server::conn::auto` with `http1.header_read_timeout(10s)` via `serve::run` (replaces `axum::serve` which doesn't surface the knob); covered by `tests/rate_limit.rs::slow_loris_header_read_timeout` | GREEN  |

### Secrets management (TM-SECRET)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-SECRET.1 | `/etc/claude-phone/gateway.toml` mode 0640 root:claude-phone                                | GREEN (`GatewayConfig::load` checks `mode & 0o027 != 0` and `bail!`s with a `chmod 640 + chown root:claude-phone` hint that names the catalog row; mirrors TM-SECRET.10. Unix-only, Windows relies on user-profile ACL. Forward-looking `tm_secret_1` test module in `config_test.rs` exercises 0644/0660/0641/0604 reject and 0640/0600/0440 accept.) |
| TM-SECRET.2 | `Zeroizing<String>` inner type for SessionToken / ApiKey                                    | GREEN  |
| TM-SECRET.3 | Manual Debug returning `"(***)"` on secret newtypes                                         | GREEN  |
| TM-SECRET.4 | Hidden server header `Server: claude-phone`                                                 | GREEN  |
| TM-SECRET.5 | Redacted PairResponse Debug (url/token/qr_ascii)                                            | GREEN  |
| TM-SECRET.6 | redact_path in TraceLayer for `/s/<token>` and `/api/phone/<token>`                         | GREEN  |
| TM-SECRET.7 | main.rs `has_public_url: bool` instead of `?public_url`                                     | GREEN  |
| TM-SECRET.8 | Pre-commit gitleaks scan; blocks commit on secret-shaped strings                            | TODO   |
| TM-SECRET.9 | `*-dev.toml` gitignored                                                                     | GREEN  |
| TM-SECRET.10 | Wrapper user config file perms 0600 enforced on load                                        | GREEN (`WrapperConfig::load` checks `mode & 0o077 != 0` and `bail!`s with a chmod hint; Unix-only, Windows relies on user-profile ACL) |
| TM-SECRET.11 | Opaque error chain in gateway responses (no internal detail in 4xx/5xx body)                | TODO   |
| TM-SECRET.12 | gateway-dev.toml placeholder (invalid length → fail-loud)                                   | GREEN  |
| TM-SECRET.13 | git history scrub: verify no historical secret-shaped commits                                | TODO   |
| TM-SECRET.14 | Wrapper log file perms 0600 (peer IPs, RPC URL, error contexts)                              | GREEN (`init_file_logging` opens both the probe and the tracing writer via a shared `restricted_open` helper that sets Unix `mode(0o600)`; rotation-on-the-fly re-applies the mode) |

### Supply chain (TM-SUPPLY)

| ID          | Mitigation                                                                                  | Status |
|-------------|---------------------------------------------------------------------------------------------|--------|
| TM-SUPPLY.1 | cargo-audit + npm-audit in CI                                                               | GREEN  |
| TM-SUPPLY.2 | Cargo.lock + package-lock.json committed (lockfile pinning)                                 | GREEN  |
| TM-SUPPLY.3 | Dependabot weekly for cargo + npm + github-actions                                          | GREEN  |
| TM-SUPPLY.4 | `cargo-deny` config (licenses, advisories via cargo-audit, bans wildcards, sources allowlist) | GREEN  |
| TM-SUPPLY.5 | Subresource Integrity (SRI) for any CDN assets — verify all self-hosted                     | VERIFY |

### Infrastructure (TM-INFRA)

| ID          | Mitigation                                                                                  | Status |
|-------------|---------------------------------------------------------------------------------------------|--------|
| TM-INFRA.1  | systemd hardening block (NoNewPrivileges, ProtectSystem=strict, ProtectHome, PrivateTmp, PrivateDevices, RestrictAddressFamilies, SystemCallFilter, CapabilityBoundingSet=, MemoryDenyWriteExecute, LockPersonality) | GREEN (`deploy/systemd/claude-phone-gateway.service` adds `SystemCallFilter=@system-service ~@privileged ~@resources` with `SystemCallErrorNumber=EPERM`; remaining directives already present pre-4.9) |
| TM-INFRA.2  | ufw default deny incoming; allow 22 from trusted IPs; allow 443 from CF IP ranges only      | GREEN (`deploy/scripts/setup-ufw.sh` idempotent apply script; reads `/etc/claude-phone/ssh-allowlist` + `cf-ipv4`/`cf-ipv6`; fails-loud on missing/empty input) |
| TM-INFRA.3  | sshd hardening: PermitRootLogin no, PasswordAuthentication no, AllowUsers whitelist, MaxAuthTries 3, LoginGraceTime 30 | GREEN (`deploy/sshd/99-claude-phone.conf` drop-in installed by `deploy.sh::install_sshd_dropin` with `sshd -t` gate; AllowUsers stays in a host-specific `98-allow-users.conf` outside the repo) |
| TM-INFRA.4  | fail2ban: sshd jail + recidive jail + claude-phone jail (watches gateway auth-failure log)  | GREEN (`deploy/fail2ban/jail.local` + `filter.d/claude-phone.conf` installed by `deploy.sh::install_fail2ban`; `scripts/fail2ban_filter_test.sh` asserts the filter against a canned 4.2-shaped auth-failure JSON sample, hooked into `security_invariants.sh`) |
| TM-INFRA.5  | auditd watch on `/etc/claude-phone/`, `/opt/claude-phone/`, `/etc/systemd/system/claude-phone-gateway.service` | GREEN (`deploy/auditd/claude-phone.rules` watches config dir, install dir, unit file, and sshd drop-in with `-p wa` and per-target `-k claude-phone-*` keys; installed by `deploy.sh::install_auditd` via `augenrules --load`; every watch key pinned by `security_invariants.sh`) |
| TM-INFRA.6  | systemd LimitNOFILE, MemoryMax, CPUQuota                                                    | GREEN (`deploy/systemd/claude-phone-gateway.service` `LimitNOFILE=8192`, `MemoryMax=256M`, `CPUQuota=80%`) |
| TM-INFRA.7  | Cloudflare WAF rules (Free tier): block common-scan paths (/.git, /.env, /wp-admin)         | GREEN (operator runbook in `deploy/cloudflare/README.md` "Custom WAF rules (TM-INFRA.7)" section: 3 rules — `block-scan-paths`, `block-unknown-api`, `rate-wrapper`; rule names + section header pinned by `security_invariants.sh`; deliberately operator-applied, not automated — see README rationale on broader WAF API token scope) |
| TM-INFRA.8  | systemd LimitCORE=0 (no core dumps)                                                         | GREEN (`deploy/systemd/claude-phone-gateway.service` `LimitCORE=0`) |
| TM-INFRA.9  | systemd journal persistence; ReadWritePaths=/var/lib/claude-phone                           | GREEN (`deploy/journald/99-claude-phone.conf` drop-in: `Storage=persistent`, `SystemMaxUse=512M`, `SystemKeepFree=2G`, `MaxRetentionSec=30day`, `ForwardToSyslog=yes`; installed by `deploy.sh::install_journald` which also creates `/var/log/journal` and restarts `systemd-journald`; `ReadWritePaths=/var/lib/claude-phone` already on the unit since TM-INFRA.1) |
| TM-INFRA.10 | Caddy ↔ Gateway on 127.0.0.1 loopback only (kernel-enforced)                                | GREEN (`deploy/scripts/post_deploy_verify.sh` TM-INFRA.10 block runs first because it's host-local; `ss -tlnp \| grep -F claude-phone-gateway` extracts the bind addr and accepts only `127.0.0.1:` or `[::1]:`; under `STRICT=1` a non-loopback bind aborts the deploy; block presence + `ss` probe + loopback pattern pinned by `security_invariants.sh`) |
| TM-INFRA.11 | `claude-phone` user non-root, no shell, owned dirs read-only via systemd ReadOnlyPaths      | GREEN (`deploy.sh` creates `claude-phone` user with `/sbin/nologin`; `deploy/systemd/claude-phone-gateway.service` adds `ReadOnlyPaths=/opt/claude-phone`) |

### Frontend (TM-FRONT)

| ID          | Mitigation                                                                                  | Status |
|-------------|---------------------------------------------------------------------------------------------|--------|
| TM-FRONT.1  | CSP nonce-based `style-src` (replace `'unsafe-inline'`)                                     | DEFER (P2) |
| TM-FRONT.2  | Trusted Types policy (require-trusted-types-for 'script')                                   | DEFER (P2) |
| TM-FRONT.3  | `history.replaceState()` to strip token from URL bar after first load                       | GREEN (`SessionPage.tsx` captures `params.token` into local state on first render and a single-shot `useEffect` calls `window.history.replaceState({}, '', '/')`; `tests/SessionPage.test.tsx` spies on `replaceState` and asserts both that it fires and that no call carries the token in the replacement URL; the WS-stays-alive test pins that the URL swap does not break the live session) |
| TM-FRONT.4  | Service worker minimal scope (`/`), explicit bypass for `/api/*`, `/s/<token>`              | VERIFY |
| TM-FRONT.5  | localStorage / sessionStorage / history-state enforcement (no token persisted)              | GREEN  |
| TM-FRONT.6  | CSP `default-src 'self'`, `frame-ancestors 'none'`, `object-src 'none'`, `base-uri 'self'`, `form-action 'self'`, `script-src 'self'` | GREEN |
| TM-FRONT.7  | Permissions-Policy `geolocation=(), microphone=(), camera=()`                               | GREEN  |
| TM-FRONT.8  | X-Frame-Options DENY                                                                        | GREEN  |
| TM-FRONT.9  | X-Content-Type-Options nosniff                                                              | GREEN  |
| TM-FRONT.10 | Subresource Integrity for any external scripts (currently zero — verify)                    | VERIFY |
| TM-FRONT.11 | Disable autofill on session input fields (`autocomplete="off"`)                             | GREEN (`InputBar.tsx` and `PasteModal.tsx` set `autoComplete="off"` plus the `autoCapitalize`/`autoCorrect`/`spellCheck` cluster; `tests/InputBar.test.tsx` and `tests/PasteModal.test.tsx` each pin the full cluster via `getAttribute` — a regression that removes any one attribute fails) |
| TM-FRONT.12 | Cross-Origin-Opener-Policy `same-origin`                                                    | DEFER (P2) |
| TM-FRONT.13 | Cross-Origin-Embedder-Policy `require-corp`                                                 | DEFER (P2) |

### WebSocket-specific (TM-WS)

| ID         | Mitigation                                                                                  | Status  |
|------------|---------------------------------------------------------------------------------------------|---------|
| TM-WS.1    | Origin enforcement on `/api/wrapper`                                                        | GREEN   |
| TM-WS.2    | Origin enforcement on `/api/phone/:token`                                                   | GREEN   |
| TM-WS.3    | Fail-closed on **missing** Origin when `public_origin` configured                           | GREEN (phone_ws.rs, fail-closed on missing Origin) |
| TM-WS.4    | `max_message_size = 64 KB`                                                                   | GREEN   |
| TM-WS.5    | `max_frame_size = 64 KB`                                                                     | GREEN   |
| TM-WS.6    | 30 s server-initiated Ping keepalive                                                         | GREEN   |
| TM-WS.7    | 60 s no-pong → drop socket (post-hello idle)                                                 | GREEN (deduplicates [[TM-RATE.7]] — the post-hello watchdog tracks `last_pong_ms` and cancels via `session.cancel.cancel()` after `PONG_DEADLINE` (90 s) in both `wrapper_ws.rs` and `phone_ws.rs`. Final deadline relaxed from the threat-model's first-pass 60 s to 90 s to absorb mobile-network jitter without false-killing healthy sessions; `tests/rate_limit.rs::pong_deadline_is_bounded_and_reasonable` pins the constant) |
| TM-WS.8    | WS compression OFF (`permessage-deflate` not negotiated)                                     | GREEN (tests/websocket.rs raw-TCP upgrade asserts server never echoes `permessage-deflate` on either route) |
| TM-WS.9    | `public_origin` fail-loud at gateway start if not configured in production                   | GREEN (config.rs validate(), Environment::Production requires public_origin) |
| TM-WS.10   | HTTP upgrade-phase timeout (axum/hyper) — explicit configured limit                          | GREEN (deduplicates [[TM-RATE.9]] — `serve::run` builds the listener with `hyper_util::server::conn::auto` and `http1.header_read_timeout(10 s)`, replacing `axum::serve` which doesn't surface the knob; `tests/rate_limit.rs::slow_loris_header_read_timeout` proves the slow-loris client gets dropped before completing headers) |
| TM-WS.11   | Strict token length on `/api/phone/:token` path before allocation                            | GREEN   |
| TM-WS.12   | WS subprotocol negotiation — strict match or unset                                           | GREEN (tests/websocket.rs raw-TCP upgrade asserts server never echoes `Sec-WebSocket-Protocol` on either route) |

### Code-level audit (TM-CODE)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-CODE.1  | `cargo clippy --all-targets -- -D warnings -W clippy::pedantic` clean (fix real issues)     | TODO   |
| TM-CODE.2  | `unsafe` block count = 0 in workspace                                                       | GREEN  |
| TM-CODE.3  | `panic!` / `unwrap` / `expect` audit in hot paths (tests-only allowed)                       | GREEN  |
| TM-CODE.4  | TOCTOU sweep in `session/registry.rs` (insert/check/use sequences)                          | GREEN  |
| TM-CODE.5  | Channel back-pressure verify: no unbounded channels in hot paths; bounded with timeout      | GREEN (`scripts/check_unbounded_channels.sh` greps every `crates/*/src/**/*.rs` for `unbounded_channel` / `unbounded_send` outside `#[cfg(test)]` blocks and fails the build on any production hit; wired as a dedicated CI step `TM-CODE.5 — unbounded-channel gate` in `.github/workflows/ci.yml`. Sink writes use `SINK_SEND_TIMEOUT = 5 s` over bounded mpsc(256) channels — see [[TM-RATE.6]]) |
| TM-CODE.6  | Integer overflow: bounds on `session_idle_timeout_secs`, `max_sessions`; `cols`/`rows` u16 OK | GREEN |
| TM-CODE.7  | Workspace `cargo fmt --check` clean                                                          | GREEN  |

### Testing (TM-TEST)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-TEST.1  | cargo-fuzz target on ControlMessage / SessionToken / ApiKey parsing                          | GREEN (`crates/claude-phone-fuzz/fuzz_targets/{control_message,session_token,api_key}.rs`; nightly smoke job `.github/workflows/fuzz-smoke.yml` runs each target for 60 s on schedule) |
| TM-TEST.2  | proptest: arbitrary ControlMessages → no panic, no infinite loop                             | GREEN (`crates/claude-phone-shared/tests/proptest_protocol.rs` covers `ControlMessage` round-trip + arbitrary input; `tests/token_test.rs` covers `SessionToken`/`ApiKey` parse fuzz) |
| TM-TEST.3  | Negative-path test matrix per guard (rejects invalid X)                                      | GREEN (`crates/claude-phone-gateway/tests/negative_path_test.rs` adds gap-filling tests per guard; combined with prior coverage in `tests/e2e_test.rs` and `tests/websocket.rs`) |
| TM-TEST.4  | Pentest-style e2e: malformed frames, oversized hello, replay token, cross-session leak, Origin spoof, concurrent same-token | GREEN (`crates/claude-phone-gateway/tests/pentest_e2e.rs` — 6 adversarial scenarios spanning all six attacker patterns named in the row) |
| TM-TEST.5  | Forward-looking grep tests: tracing patterns, derived-Debug patterns                          | GREEN (scripts/check_tracing_secrets.sh + scripts/check_debug_derive_secrets.sh, wired into CI rust job) |
| TM-TEST.6  | Chaos test: random kill -9 wrapper / phone, gateway recovery + session cleanup                | GREEN (`crates/claude-phone-gateway/tests/chaos_test.rs` — 3 forward-looking invariants: wrapper-drop releases token, paired wrapper+phone drop reclaims slot ≤1 s, deterministic-seed chaos with N=6 random drops; backed by `wrapper_ws.rs` + `phone_ws.rs` cancel-propagation fix landed in commit `51a9065`) |

### Cross-cutting / Leakage (TM-LEAK)

| ID         | Mitigation                                                                                  | Status |
|------------|---------------------------------------------------------------------------------------------|--------|
| TM-LEAK.1  | CI grep: tracing macros with secret-named fields fail build                                  | GREEN (scripts/check_tracing_secrets.sh, CI rust job; safe-marker `// TM-LEAK.1: safe — <reason>`) |
| TM-LEAK.2  | CI grep heuristic: derived Debug on container with secret-named field flagged for review     | GREEN (scripts/check_debug_derive_secrets.sh, CI rust job; field-type or block-comment escape hatch) |
| TM-LEAK.3  | CI grep / list: asymmetric guards across routes (Origin missing on routes that should have it) | GREEN (scripts/asymmetric_guards.sh wired into CI via scripts/security_invariants.sh) |
| TM-LEAK.4  | TM-to-code coverage matrix: every TM-CAT.N appears as comment in code or test               | GREEN (`scripts/tm_coverage.sh` runs both directions — catalog→references and references→catalog — wired into `scripts/security_invariants.sh` aggregator and as its own named step in `.github/workflows/ci.yml`; 19 anchor annotations added across `crates/`, `web/`, `deploy/`, `scripts/` so every non-TODO/non-DEFER row has a code reference) |

---

## 14. Change log

- 2026-05-23 — Initial draft (brainstorming session 2026-05-23). Pending user approval.

---

## 15. References

- Pre-step regression sweep (2026-05-23) — verified Round 1-3 mitigations in place; partial gap on phone-side upgrade-phase slow-loris
- `docs/security.md` v1 — superseded; will become pointer + summary after approval
- `docs/superpowers/specs/2026-05-22-test-coverage-and-leakage-prevention-design.md`
- `docs/protocol.md` — WS protocol
- `docs/deployment.md` — production deploy notes
- `docs/adr/*` — architecture decision records
- Commits: `be60102` (R1), `7d1fd1d` (R2), `6e5c63d` (R3)
