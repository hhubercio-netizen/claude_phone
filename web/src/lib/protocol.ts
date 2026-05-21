// Mirror of crates/claude-phone-shared/src/protocol.rs
// Keep in sync manually (or use a codegen tool in v2).

export type SessionToken = string;  // 43 base64url chars
export type ApiKey = string;        // same shape

export type ErrorCode =
  | 'invalid_token'
  | 'invalid_api_key'
  | 'session_taken'
  | 'expired'
  | 'internal'
  | 'protocol_violation';

export type ControlMessage =
  | { type: 'wrapper_hello'; api_key: ApiKey; token: SessionToken; cols: number; rows: number; claude_version?: string }
  | { type: 'phone_hello'; token: SessionToken; cols: number; rows: number; user_agent?: string }
  | { type: 'server_hello'; session_id: string; peer_connected: boolean }
  | { type: 'error'; code: ErrorCode; message: string }
  | { type: 'resize'; cols: number; rows: number }
  | { type: 'peer_status'; connected: boolean }
  | { type: 'close'; reason?: string };

export function parseControlMessage(raw: string): ControlMessage {
  const obj = JSON.parse(raw);
  if (typeof obj !== 'object' || obj === null || typeof obj.type !== 'string') {
    throw new Error('not a control message');
  }
  return obj as ControlMessage;
}

export function encodeControlMessage(msg: ControlMessage): string {
  return JSON.stringify(msg);
}
