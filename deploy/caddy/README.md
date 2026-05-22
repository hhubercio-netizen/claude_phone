# Caddy in front of claude-phone-gateway

Caddy terminates TLS on the local server and proxies HTTP + WS to the gateway
on `127.0.0.1:8080`.

## Cloudflare arrangement

Two reasonable options:

1. **Cloudflare proxied A record** — fastest setup. Set DNS to your home IP
   (DDNS works), enable orange cloud. Cloudflare ↔ origin TLS = "Full (strict)"
   if your origin cert is publicly trusted, or "Full" if self-signed.
2. **Cloudflare Tunnel** — no port-forward, no public IP needed. Install
   `cloudflared`, create a tunnel, point it at `localhost:443` (or `localhost:80`
   if you skip local TLS). Best for residential ISPs that block ports 80/443.

WebSocket is supported in both arrangements — Cloudflare has WS passthrough.

## Certs

Two options:

- Use Let's Encrypt via Caddy's automatic HTTPS (remove `tls /etc/...` line
  and the `auto_https off` directive, and let Caddy do ACME). Requires
  port 80 reachable from the internet for HTTP-01 challenge, OR DNS-01
  with a Cloudflare API token (add the cloudflare module to Caddy).
- Use a Cloudflare Origin Certificate (free, 15-year). Place at
  `/etc/caddy/cert.pem` and `/etc/caddy/key.pem`. This is what the Caddyfile
  in this directory assumes.
