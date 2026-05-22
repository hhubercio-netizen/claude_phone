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
