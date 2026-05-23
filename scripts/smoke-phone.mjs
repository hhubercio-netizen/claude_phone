// Simulate the phone side: open WS to the gateway, send phone_hello, type
// `ls /; echo MARKER-$$\n`, then dump every binary chunk that comes back
// until we see the marker (or 10s timeout).
//
// Usage:
//   node scripts/smoke-phone.mjs <token>
//   PHONE_TOKEN=<token> node scripts/smoke-phone.mjs
//   GATEWAY_URL=ws://1.2.3.4:9090 node scripts/smoke-phone.mjs <token>
import WebSocket from 'ws';

// Accept the token from any arg position so leading `-` doesn't get eaten by
// the shell as a flag. Fall back to env var.
const token =
  process.argv.slice(2).find((a) => /^[A-Za-z0-9_-]{43}$/.test(a)) ||
  process.env.PHONE_TOKEN;
if (!token) {
  console.error('usage: node scripts/smoke-phone.mjs <token>   (or PHONE_TOKEN=...)');
  process.exit(2);
}

const gateway = process.env.GATEWAY_URL || 'ws://127.0.0.1:8080';
const url = `${gateway}/api/phone/${token}`;
const ws = new WebSocket(url);
const marker = `MARKER-${process.pid}-${Date.now()}`;
let received = '';
let serverHelloSeen = false;

const timeout = setTimeout(() => {
  console.error('\n[FAIL] timed out without seeing marker');
  console.error(`bytes received: ${received.length}`);
  console.error('last 200 chars:', JSON.stringify(received.slice(-200)));
  process.exit(1);
}, 10000);

ws.on('open', () => {
  console.error('[open] WS connected');
  ws.send(JSON.stringify({
    type: 'phone_hello',
    token,
    cols: 80,
    rows: 24,
    user_agent: 'smoke-test/1.0',
  }));
});

ws.on('message', (data, isBinary) => {
  if (isBinary) {
    const text = data.toString('utf8');
    received += text;
    process.stdout.write(text);
    if (received.includes(marker)) {
      clearTimeout(timeout);
      console.error('\n[OK] saw marker — PTY is alive and the bridge round-trips bytes');
      ws.close();
      process.exit(0);
    }
  } else {
    const msg = JSON.parse(data.toString('utf8'));
    console.error('[control]', msg);
    if (msg.type === 'server_hello') {
      serverHelloSeen = true;
      // Send keystrokes a moment after hello so bash has its prompt drawn.
      setTimeout(() => {
        const cmd = `ls /; echo ${marker}\n`;
        ws.send(Buffer.from(cmd, 'utf8'));
        console.error('[sent] keystrokes:', JSON.stringify(cmd));
      }, 500);
    }
  }
});

ws.on('error', (e) => {
  console.error('[error]', e.message);
  process.exit(1);
});

ws.on('close', (code, reason) => {
  console.error(`[close] code=${code} reason=${reason}`);
  if (!serverHelloSeen) process.exit(1);
});
