# WebSocket protocol

The wrapper ↔ gateway and gateway ↔ phone WebSockets carry the same protocol.
Two frame kinds:

- **Text frames**: JSON-encoded control messages (`ControlMessage` discriminated
  union — Rust enum in `crates/claude-phone-shared/src/protocol.rs`, mirrored as
  a TypeScript union in `web/src/lib/protocol.ts`).
- **Binary frames**: opaque PTY bytes. Wrapper writes its PTY's stdout into
  these, gateway forwards to phone, phone writes them straight into xterm.js.
  In reverse, the phone's keystrokes go in binary frames to the wrapper, which
  writes them to the PTY's stdin.

## Control messages

All shapes are `{ "type": "...", ... }` — `serde(tag = "type", rename_all = "snake_case")`.

### `wrapper_hello`

First frame on `/api/wrapper`. Authenticates the wrapper with its API key and
registers a session token.

```typescript
{ type: 'wrapper_hello',
  api_key: string,       // 43 base64url chars
  token: string,         // 43 base64url chars (the session token)
  cols: number,          // initial PTY width
  rows: number,          // initial PTY height
  claude_version?: string }
```

Rust: `claude_phone_shared::protocol::WrapperHello`.

### `phone_hello`

First frame on `/api/phone/:token`. Confirms which session the phone wants
(token from URL path is the source of truth; the JSON `token` is sanity-check).

```typescript
{ type: 'phone_hello',
  token: string,         // must equal path :token
  cols: number,
  rows: number,
  user_agent?: string }
```

### `server_hello`

Server's response after a successful hello.

```typescript
{ type: 'server_hello',
  session_id: string,    // internal session id for debugging
  peer_connected: boolean }
```

### `error`

Server's response on any failure.

```typescript
{ type: 'error',
  code: 'invalid_token' | 'invalid_api_key' | 'session_taken' | 'expired' | 'internal' | 'protocol_violation',
  message: string }
```

### `resize`

Sent by phone (or wrapper, but rare) to update PTY dimensions.

```typescript
{ type: 'resize', cols: number, rows: number }
```

### `peer_status`

Server notifies one side that the other connected/disconnected.

```typescript
{ type: 'peer_status', connected: boolean }
```

### `close`

Graceful shutdown signal.

```typescript
{ type: 'close', reason?: string }
```

## Sequence: full pairing

```
wrapper                gateway                  phone
   |                      |                       |
   |  WS upgrade          |                       |
   |--------------------->|                       |
   |  wrapper_hello       |                       |
   |--------------------->|                       |
   |  server_hello        |                       |
   |<---------------------|                       |
   |                      |                       |
   |                      |  WS upgrade           |
   |                      |<----------------------|
   |                      |  phone_hello          |
   |                      |<----------------------|
   |                      |  server_hello         |
   |                      |---------------------->|
   |  peer_status(true)   |                       |
   |<---------------------|                       |
   |                      |                       |
   |  <-- bidirectional binary frames + occasional resize -->
   |                      |                       |
```

## Frame size limits

Gateway should cap text frames at 4 KB and binary frames at 64 KB (not yet
enforced — tracked as future work in `docs/security.md`).

## Keepalive

Native WebSocket ping/pong is used for keepalive. No application-level heartbeat
frames.

## Cross-language drift

The Rust enum and the TS union are hand-maintained. When you add or change a
message, update both files. Tests in both crates verify their own roundtrips,
but no cross-language conformance test exists in v1.
