// Sticky-session smoke: phone connects, types a marker, closes; then a NEW
// phone connects on the SAME token and types a *different* marker. The
// second phone must see its marker echo back — proving:
//   1. The wrapper kept the PTY alive after peer disconnect (didn't die on
//      peer_status connected=false).
//   2. The gateway accepts a reattach on the same token (sticky session).
//   3. The freshly-attached phone gets a working bidirectional bridge.
//
// Usage:
//   PHONE_TOKEN=<token> node scripts/smoke-reconnect.mjs
//   GATEWAY_URL=ws://127.0.0.1:8080 node scripts/smoke-reconnect.mjs

import WebSocket from 'ws';

const token =
  process.argv.slice(2).find((a) => /^[A-Za-z0-9_-]{43}$/.test(a)) ||
  process.env.PHONE_TOKEN;
if (!token) {
  console.error('usage: PHONE_TOKEN=<token> node scripts/smoke-reconnect.mjs');
  process.exit(2);
}

const gateway = process.env.GATEWAY_URL || 'ws://127.0.0.1:8080';
const url = `${gateway}/api/phone/${token}`;

function runPhase(label, marker) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    let received = '';
    let serverHello = false;
    const timeout = setTimeout(() => {
      ws.terminate();
      reject(
        new Error(`[${label}] timeout (got ${received.length} bytes): ${JSON.stringify(received.slice(-200))}`),
      );
    }, 8000);

    ws.on('open', () => {
      console.error(`[${label}] WS open`);
      ws.send(
        JSON.stringify({
          type: 'phone_hello',
          token,
          cols: 80,
          rows: 24,
          user_agent: 'smoke-reconnect/1.0',
        }),
      );
    });

    ws.on('message', (data, isBinary) => {
      if (isBinary) {
        received += data.toString('utf8');
        if (received.includes(marker)) {
          clearTimeout(timeout);
          console.error(`[${label}] saw '${marker}'`);
          ws.once('close', () => resolve());
          ws.close();
        }
      } else {
        const msg = JSON.parse(data.toString('utf8'));
        if (msg.type === 'server_hello') {
          serverHello = true;
          // Let bash draw its prompt before sending input.
          setTimeout(() => {
            ws.send(Buffer.from(`echo ${marker}\n`, 'utf8'));
          }, 500);
        }
      }
    });

    ws.on('error', (e) => {
      clearTimeout(timeout);
      reject(new Error(`[${label}] ws error: ${e.message}`));
    });
    ws.on('close', () => {
      if (!serverHello) {
        clearTimeout(timeout);
        reject(new Error(`[${label}] closed before server_hello`));
      }
    });
  });
}

async function main() {
  const marker1 = `RECONNECT-A-${process.pid}-${Date.now()}`;
  const marker2 = `RECONNECT-B-${process.pid}-${Date.now()}`;

  console.error('[phase1] connecting first phone');
  await runPhase('phase1', marker1);

  // Give the gateway a moment to register the WS close and detach the phone.
  console.error('[gap] waiting 500ms with no phone attached');
  await new Promise((r) => setTimeout(r, 500));

  console.error('[phase2] reconnecting on same token');
  await runPhase('phase2', marker2);

  console.error('[OK] sticky session: same link worked twice in a row');
  process.exit(0);
}

main().catch((e) => {
  console.error('[FAIL]', e.message);
  process.exit(1);
});
