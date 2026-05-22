# Gateway HTTP API

Base URL (production): `https://claude-phone.pl`

## `GET /healthz`

Liveness probe. Returns `200 OK` with JSON body:

```json
{ "status": "ok", "version": "0.1.0" }
```

Used by `deploy/scripts/update.sh` to verify the gateway is alive after a swap.

## `GET /`

Returns the React app's `index.html`. No special behaviour — used so the root
URL doesn't 404.

## `GET /s/:token`

Returns the React app's `index.html`. React Router takes over client-side and
routes to `SessionPage`, which uses `token` from the URL to open a phone
WebSocket.

`token` must be a 43-character base64url string. Anything else falls through to
the React 404 page; no server-side validation here (the WS upgrade endpoint
does the real validation).

## `GET /assets/*`

Cached, fingerprinted Vite bundles (JS, CSS, fonts). Long-cache headers. Hashes
in filenames mean we can cache aggressively (`Cache-Control: public, max-age=31536000, immutable`).

## `WS /api/wrapper`

WebSocket upgrade endpoint for the local wrapper.

**Handshake:** within 5 seconds of upgrade, the client sends a JSON text frame
matching `WrapperHello`:

```json
{
  "type": "wrapper_hello",
  "api_key": "<43-char base64url>",
  "token": "<43-char base64url>",
  "cols": 80,
  "rows": 24,
  "claude_version": "1.2.3"
}
```

Server validates the API key (constant-time compare against allowlist) and
checks the session token isn't already taken, then responds with:

```json
{ "type": "server_hello", "session_id": "abc123", "peer_connected": false }
```

Or, on failure:

```json
{ "type": "error", "code": "invalid_api_key", "message": "unknown api key" }
```

After the handshake, bytes flow bidirectionally — see `docs/protocol.md`.

## `WS /api/phone/:token`

WebSocket upgrade endpoint for the phone.

**Handshake:** client sends `PhoneHello` with the same `token` from the URL
path:

```json
{
  "type": "phone_hello",
  "token": "<same as path>",
  "cols": 40,
  "rows": 80,
  "user_agent": "Mozilla/5.0 ..."
}
```

Server looks up the session by token; if a wrapper is registered with it,
attaches the phone and responds with `server_hello` (and notifies the wrapper
via a `peer_status` frame). If no session exists, responds with
`{ "type": "error", "code": "invalid_token", ... }` and closes.

## Headers

The gateway emits:

- `Strict-Transport-Security: max-age=31536000; includeSubDomains`
- `Referrer-Policy: no-referrer`
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `Permissions-Policy: interest-cohort=()`

(Caddy adds these via `header { ... }` block — see `deploy/caddy/Caddyfile`.)

## Rate limits

Not enforced at the gateway; recommended at Cloudflare:

- `/api/wrapper`: 10 connection attempts/min/IP (defensive).
- `/api/phone/*`: not rate-limited (token entropy is the gate).
