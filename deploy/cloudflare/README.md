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
