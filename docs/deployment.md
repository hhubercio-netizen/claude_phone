# Deploying the gateway

This guide walks through deploying `claude-phone-gateway` on an Ubuntu
22.04+ server fronted by Cloudflare on the `claude-phone.pl` domain.

## Prerequisites

- A domain (`claude-phone.pl`) with DNS managed by Cloudflare.
- An Ubuntu server reachable on ports 80/443 (or a Cloudflare Tunnel set up).
- Rust toolchain (pinned by `rust-toolchain.toml` — currently 1.87.0).
- Node.js 20+ for the frontend build.

## Step 1: Cloudflare DNS + Origin Cert

1. In Cloudflare dashboard, set DNS A record for `claude-phone.pl` → your
   server IP, orange cloud (proxied).
2. SSL/TLS mode: **Full (strict)**.
3. SSL/TLS → Origin Server → Create Certificate. Hostnames:
   `claude-phone.pl, *.claude-phone.pl`. Validity 15 years. RSA 2048 or ECDSA.
4. Save the cert and key to your server:
   - `/etc/caddy/cert.pem` (paste the certificate body)
   - `/etc/caddy/key.pem` (paste the private key)
   Mode `0600`, owner `caddy:caddy` (after Caddy install).

See `deploy/cloudflare/README.md` for cache rules and WAF tips.

## Step 2: Install Caddy

```bash
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | \
    sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | \
    sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update
sudo apt install -y caddy
```

Copy `deploy/caddy/Caddyfile` to `/etc/caddy/Caddyfile`. Reload:

```bash
sudo systemctl reload caddy
```

## Step 3: Clone repo and deploy

```bash
sudo mkdir -p /opt
sudo git clone https://github.com/YOU/claude-phone /opt/claude-phone-src
cd /opt/claude-phone-src
sudo bash deploy/scripts/deploy.sh
```

This:
- creates user `claude-phone`
- builds the gateway in release mode and the frontend bundle
- installs to `/opt/claude-phone/`
- writes `/etc/claude-phone/gateway.toml` (edit it: add an API key!)
- installs and starts the systemd unit

## Step 4: Generate API key for your wrapper

```bash
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=' | head -c 43
```

Copy the 43-char output to `/etc/claude-phone/gateway.toml` under
`api_keys = [...]`, then restart:

```bash
sudo systemctl restart claude-phone-gateway
```

Put the same key into your **dev machine's** wrapper config
(`~/.config/claude-phone/config.toml`):

```toml
gateway_url = "wss://claude-phone.pl/api/wrapper"
api_key = "PASTE_KEY_HERE"
public_url_base = "https://claude-phone.pl"
```

## Step 5: Smoke test

From the dev machine:

```bash
claude-phone --claude-bin bash
# inside, type /phone — see QR — open URL on phone
```

You should see the bash terminal mirrored on your phone.

## Updates

```bash
cd /opt/claude-phone-src
sudo bash deploy/scripts/update.sh
```

This pulls latest main, builds new artifacts in a staging dir, atomically
swaps the binary and `web/` dist, restarts the service, and runs a healthcheck
on `/healthz`. On failure it rolls back automatically.

## Manual rollback

```bash
sudo bash deploy/scripts/rollback.sh
```

(Only works while the `.old` binary is still in `/opt/claude-phone/bin/`.)

## Logs

```bash
sudo journalctl -u claude-phone-gateway -f
```

The gateway emits structured JSON when `log_format = "json"` in
`gateway.toml`, suitable for forwarding to Loki/CloudWatch/etc.

## Backup

The gateway is stateless — sessions live in memory. Nothing to back up
beyond the config in `/etc/claude-phone/gateway.toml`.

## Docker alternative

If you prefer Docker, use `deploy/docker-compose.yml`. The compose file runs
both the gateway and Caddy. Put your `gateway.toml` in the deploy dir and
your `cert.pem`/`key.pem` under `deploy/certs/`. Then:

```bash
cd deploy
docker compose up -d
```
