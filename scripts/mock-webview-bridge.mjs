#!/usr/bin/env node
// Minimal mock of the Tauri-side webview_apis WS server. Lets you curl
// `openhuman.webview_apis_gmail_*` against the core binary without
// bringing up the full Tauri shell. Usage:
//   node scripts/mock-webview-bridge.mjs 9826

import { WebSocketServer } from 'ws';

function usage() {
  return 'Usage: node scripts/mock-webview-bridge.mjs [port]';
}

function readPortArg() {
  const args = process.argv.slice(2);
  const [rawPort] = args;
  if (rawPort === '--help' || rawPort === '-h') {
    console.log(usage());
    process.exit(0);
  }
  if (args.length > 1) {
    console.error(usage());
    process.exit(2);
  }
  const port = Number(rawPort ?? 9826);
  if (!Number.isInteger(port) || port <= 0 || port >= 65536) {
    console.error('[mock-bridge] port must be an integer between 1 and 65535');
    process.exit(2);
  }
  return port;
}

const port = readPortArg();
const wss = new WebSocketServer({ host: '127.0.0.1', port });

console.log(`[mock-bridge] listening on ws://127.0.0.1:${port}`);

wss.on('connection', (sock, req) => {
  console.log(`[mock-bridge] conn from ${req.socket.remoteAddress}`);
  sock.on('message', (raw) => {
    const msg = JSON.parse(raw.toString());
    console.log(`[mock-bridge] <- ${msg.method} id=${msg.id}`);
    const reply = { kind: 'response', id: msg.id };
    switch (msg.method) {
      case 'gmail.list_labels':
        reply.ok = true;
        reply.result = [
          { id: 'INBOX',     name: 'Inbox',    kind: 'system', unread: 3 },
          { id: 'STARRED',   name: 'Starred',  kind: 'system', unread: null },
          { id: 'Receipts',  name: 'Receipts', kind: 'user',   unread: 1 },
        ];
        break;
      case 'gmail.list_messages':
        reply.ok = true;
        reply.result = [
          {
            id: 'm-001', thread_id: 't-001',
            from: 'alice@example.com', to: ['you@example.com'], cc: [],
            subject: 'Hello from the mock', snippet: 'mock snippet',
            body: null, date_ms: Date.now(),
            labels: ['INBOX'], unread: true,
          },
        ];
        break;
      default:
        reply.ok = false;
        reply.error = `mock-bridge: unhandled method '${msg.method}'`;
    }
    sock.send(JSON.stringify(reply));
    console.log(`[mock-bridge] -> ${msg.method} id=${msg.id} ok=${reply.ok}`);
  });
});
