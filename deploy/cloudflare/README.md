# Cloudflare setup for claude-phone.pl

## DNS

Create an A record (or AAAA for IPv6) for `claude-phone.pl` pointing at your
home server's public IP. Enable the orange cloud (proxied through Cloudflare).

If your home IP is dynamic, use a DDNS updater (Cloudflare's API has rate
limits but is fine for hourly updates).

## WebSocket support

Cloudflare proxies WS by default for paid plans. On the Free plan, WS works for
sites on Cloudflare's network — confirm in dashboard: Network → WebSockets: On.

## Cache rules

Disable caching for `/api/*` paths. Static React assets (`/assets/*`,
fingerprinted by Vite) can be cached aggressively. Recommended page rules:

| Path             | Cache level | Edge cache TTL |
|------------------|-------------|----------------|
| `/api/*`         | Bypass      | -              |
| `/assets/*`      | Cache       | 1 month        |
| `/s/*`           | Bypass      | -              |
| `/`              | Bypass      | -              |

(`/s/*` and `/` serve `index.html` which has no fingerprint; better to skip
cache and let the upstream gzip handle it.)

## Security

- **WAF**: enable Bot Fight Mode. Phone clients are real browsers, so they
  shouldn't be blocked. Wrapper connects from a fixed home IP — add as
  exception if needed.
- **Rate limiting**: cap `/api/wrapper` to ~10 connection attempts/min per IP
  (defensive — your wrapper will only ever connect rarely).
- **Access Rules**: optionally restrict `/api/wrapper` to your home IP (or VPN
  range) using an IP Access Rule. Phones access `/api/phone/*` from anywhere
  and need broad access.

## TLS mode (TM-TLS.8)

Set **SSL/TLS encryption mode** to **Full (strict)** under SSL/TLS → Overview.
This is the only acceptable mode for the threat model:

| Mode             | Browser ↔ CF | CF ↔ origin              | Acceptable?               |
|------------------|--------------|--------------------------|---------------------------|
| Off              | plain HTTP   | plain HTTP               | no                        |
| Flexible         | TLS          | plain HTTP               | no — origin sees cleartext |
| Full             | TLS          | TLS, cert NOT validated  | no — MITM CF↔origin possible |
| **Full (strict)** | TLS          | TLS, cert validated      | yes — required             |

`deploy/scripts/post_deploy_verify.sh` checks the active mode via the
Cloudflare API on every deploy. Provide the two env vars before running
`deploy/scripts/deploy.sh`:

```
export CF_API_TOKEN=<token with Zone:SSL and Certificates:Read>
export CF_ZONE_ID=<zone id, visible on the Overview tab>
```

Create the token in CF dashboard → My Profile → API Tokens → Create Token →
Custom token. Permissions: `Zone — SSL and Certificates — Read`. Zone
Resources: `Include — Specific zone — claude-phone.pl`. Store the resulting
token in `/etc/claude-phone/cf.env` (mode 0600, owned by root) and source
it before `deploy.sh`:

```
sudo install -m 0600 -o root -g root /dev/stdin /etc/claude-phone/cf.env <<'EOF'
export CF_API_TOKEN=...
export CF_ZONE_ID=...
EOF

sudo bash -c 'source /etc/claude-phone/cf.env && /opt/claude-phone-src/deploy/scripts/deploy.sh'
```

Without those env vars the verify script soft-fails with a warning;
under `STRICT=1` (the production deploy default) the deploy aborts.

## Custom WAF rules (TM-INFRA.7)

Cloudflare Free tier allows up to **5 custom WAF rules**. We use **3**,
leaving 2 for the operator's future use. Apply via the dashboard:
**Security → WAF → Custom rules → Create rule**. Each rule is given here
as the expression you paste into the dashboard's expression editor (the
"edit expression" toggle, not the visual builder).

These rules are **operator-applied**, not automated. Reason: the CF API
token scope required for WAF rule CRUD is much broader (`Zone — WAF
Configuration — Edit`) than the read-only `Zone — SSL and Certificates —
Read` token used for TM-TLS.8. A leaked WAF-edit token would let an
attacker disable our protections; the read-only TLS token cannot.

| # | Name              | Expression                                                                                                                                                                                                  | Action       |
|---|-------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|
| 1 | block-scan-paths  | `(http.request.uri.path contains "/.git" or http.request.uri.path contains "/.env" or http.request.uri.path contains "/wp-admin" or http.request.uri.path contains "/vendor/phpunit" or http.request.uri.path contains "/xmlrpc.php")` | Block        |
| 2 | block-unknown-api | `starts_with(http.request.uri.path, "/api/") and not http.request.uri.path eq "/api/wrapper" and not starts_with(http.request.uri.path, "/api/phone/")`                                                     | Block        |
| 3 | rate-wrapper      | `http.request.uri.path eq "/api/wrapper"` — configure as a **Rate Limiting Rule** (not WAF Custom): threshold **10 requests / 1 minute / per IP**, action **Block for 1 hour**                              | Block (1 h)  |

Notes per rule:

- **Rule #1** stops common reconnaissance scans at the edge — saves origin
  CPU and pre-empts noisy 404s in our auth-failure log (which would
  otherwise be poisoned with bot traffic and reduce the signal fail2ban
  TM-INFRA.4 sees).
- **Rule #2** enforces our endpoint allow-list at the CDN layer. It is a
  defense-in-depth mirror of the origin route mux: any `/api/*` path
  outside the two known routes gets blocked before reaching the origin.
  **Adding a new `/api/*` route in the gateway requires updating this
  rule** — deliberate friction so that new public endpoints get reviewed.
- **Rule #3** is the CDN-layer mirror of TM-RATE.1 (`tower_governor`
  per-IP) and TM-RATE.2 (`AuthRateLimiter` per-IP lockout). Even if a
  flood reaches CF, it gets stopped before consuming origin sockets.

Verification: after applying, run
`curl -I https://claude-phone.pl/.git/config` from a non-allowlisted IP —
the response should be a Cloudflare 403, not a 404 from the gateway.
A 404 indicates the WAF rule was not applied or the path syntax differs.

