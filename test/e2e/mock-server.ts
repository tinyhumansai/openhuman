/* eslint-disable */
// @ts-nocheck
/**
 * Local HTTP mock server for E2E tests.
 *
 * Replaces the real backend so login-flow tests are fully self-contained.
 * Uses only `node:http` and `node:crypto` — no extra npm dependencies.
 *
 * Also handles WebSocket upgrades for the Socket.IO/Engine.IO endpoint so the
 * Rust-native socket manager doesn't crash from repeated connection failures.
 *
 * Default port: 18473 (high ephemeral, avoids Vite 1420 / Appium 4723 / backend 5005).
 */
import crypto from 'node:crypto';
import http from 'node:http';

const DEFAULT_PORT = 18_473;

// ---------------------------------------------------------------------------
// Request log
// ---------------------------------------------------------------------------

let requestLog = [];

export function getRequestLog() {
  return [...requestLog];
}

export function clearRequestLog() {
  requestLog = [];
}

// ---------------------------------------------------------------------------
// Mock data — shapes taken from src/test/handlers.ts (MSW unit-test mocks)
// ---------------------------------------------------------------------------

const MOCK_JWT = 'e2e-mock-jwt-token';

const MOCK_USER = {
  _id: 'test-user-123',
  telegramId: 12345678,
  hasAccess: true,
  magicWord: 'alpha',
  firstName: 'Test',
  lastName: 'User',
  username: 'testuser',
  role: 'user',
  activeTeamId: 'team-1',
  referral: {},
  subscription: { hasActiveSubscription: false, plan: 'FREE' },
  settings: {
    dailySummariesEnabled: false,
    dailySummaryChatIds: [],
    autoCompleteEnabled: false,
    autoCompleteVisibility: 'always',
    autoCompleteWhitelistChatIds: [],
    autoCompleteBlacklistChatIds: [],
  },
  usage: {
    cycleBudgetUsd: 10,
    spentThisCycleUsd: 0,
    spentTodayUsd: 0,
    cycleStartDate: new Date().toISOString(),
  },
  autoDeleteTelegramMessagesAfterDays: 30,
  autoDeleteThreadsAfterDays: 30,
};

// ---------------------------------------------------------------------------
// CORS helpers
// ---------------------------------------------------------------------------

const CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, PATCH, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
  'Access-Control-Max-Age': '86400',
};

function setCors(res) {
  for (const [key, value] of Object.entries(CORS_HEADERS)) {
    res.setHeader(key, value);
  }
}

function json(res, status, body) {
  setCors(res);
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

// ---------------------------------------------------------------------------
// Route handling (HTTP)
// ---------------------------------------------------------------------------

function readBody(req) {
  return new Promise(resolve => {
    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString()));
  });
}

async function handleRequest(req, res) {
  const method = req.method ?? 'GET';
  const url = req.url ?? '/';
  const body = await readBody(req);

  // Log every request for test assertions
  requestLog.push({ method, url, body, timestamp: Date.now() });

  // CORS preflight
  if (method === 'OPTIONS') {
    setCors(res);
    res.writeHead(204);
    res.end();
    return;
  }

  // Socket.IO polling transport (GET /socket.io/?EIO=4&transport=polling)
  // Respond with Engine.IO OPEN packet so polling clients don't error out.
  if (url.startsWith('/socket.io/')) {
    const eioOpen = JSON.stringify({
      sid: 'mock-sid-' + Date.now(),
      upgrades: ['websocket'],
      pingInterval: 25000,
      pingTimeout: 20000,
    });
    // Engine.IO packet type 0 = OPEN, prefixed with byte count for polling
    const packet = `${eioOpen.length + 1}:0${eioOpen}`;
    setCors(res);
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.end(packet);
    return;
  }

  // POST /telegram/login-tokens/:token/consume
  if (method === 'POST' && /^\/telegram\/login-tokens\/[^/]+\/consume\/?$/.test(url)) {
    json(res, 200, { success: true, data: { jwtToken: MOCK_JWT } });
    return;
  }

  // GET /telegram/me
  if (method === 'GET' && /^\/telegram\/me\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: MOCK_USER });
    return;
  }

  // GET /teams
  if (method === 'GET' && /^\/teams\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return;
  }

  // POST /invite/redeem
  if (method === 'POST' && /^\/invite\/redeem\/?$/.test(url)) {
    json(res, 200, { success: true, data: { message: 'Invite code redeemed successfully' } });
    return;
  }

  // GET /invite/my-codes
  if (method === 'GET' && /^\/invite\/my-codes\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return;
  }

  // GET /invite/status
  if (method === 'GET' && /^\/invite\/status/.test(url)) {
    json(res, 200, { success: true, data: { valid: true } });
    return;
  }

  // POST /telegram/settings/onboarding-complete
  if (method === 'POST' && /^\/telegram\/settings\/onboarding-complete\/?$/.test(url)) {
    json(res, 200, { success: true, data: {} });
    return;
  }

  // GET /billing/current-plan
  if (method === 'GET' && /^\/billing\/current-plan\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { plan: 'FREE', hasActiveSubscription: false, planExpiry: null, subscription: null },
    });
    return;
  }

  // Catch-all — prevents app crashes from unexpected API calls
  json(res, 200, { success: true, data: {} });
}

// ---------------------------------------------------------------------------
// WebSocket upgrade handler (minimal Engine.IO + Socket.IO)
//
// The Rust SocketManager connects via WebSocket to
//   ws://host/socket.io/?EIO=4&transport=websocket
// and expects:
//   1. Engine.IO OPEN packet  (type 0): JSON with sid, pingInterval, etc.
//   2. Socket.IO CONNECT ACK  (type 40): JSON with sid
//   3. Periodic Engine.IO PING (type 2) which we respond to with PONG (3)
//
// Without this, the Rust ws_loop retries forever and may destabilize the app.
// ---------------------------------------------------------------------------

function handleWebSocketUpgrade(req, socket, head) {
  // Only handle /socket.io/ WebSocket upgrades
  if (!req.url?.startsWith('/socket.io/')) {
    socket.destroy();
    return;
  }

  // Perform WebSocket handshake (RFC 6455)
  const key = req.headers['sec-websocket-key'];
  if (!key) {
    socket.destroy();
    return;
  }

  const acceptKey = crypto
    .createHash('sha1')
    .update(key + '258EAFA5-E914-47DA-95CA-5AB5DC085B11')
    .digest('base64');

  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\n' +
      'Upgrade: websocket\r\n' +
      'Connection: Upgrade\r\n' +
      `Sec-WebSocket-Accept: ${acceptKey}\r\n` +
      '\r\n'
  );

  const mockSid = 'mock-ws-' + Date.now();

  // Send Engine.IO OPEN packet (type 0)
  const eioOpen = JSON.stringify({
    sid: mockSid,
    upgrades: [],
    pingInterval: 25000,
    pingTimeout: 60000,
    maxPayload: 1000000,
  });
  sendWsText(socket, `0${eioOpen}`);

  // Buffer for partial frames
  let buffer = Buffer.alloc(0);

  socket.on('data', chunk => {
    buffer = Buffer.concat([buffer, chunk]);

    while (buffer.length >= 2) {
      const firstByte = buffer[0];
      const opcode = firstByte & 0x0f;
      const secondByte = buffer[1];
      const masked = (secondByte & 0x80) !== 0;
      let payloadLen = secondByte & 0x7f;
      let offset = 2;

      if (payloadLen === 126) {
        if (buffer.length < 4) return; // need more data
        payloadLen = buffer.readUInt16BE(2);
        offset = 4;
      } else if (payloadLen === 127) {
        if (buffer.length < 10) return;
        payloadLen = Number(buffer.readBigUInt64BE(2));
        offset = 10;
      }

      const maskSize = masked ? 4 : 0;
      const totalLen = offset + maskSize + payloadLen;
      if (buffer.length < totalLen) return; // need more data

      let payload = buffer.subarray(offset + maskSize, totalLen);

      if (masked) {
        const mask = buffer.subarray(offset, offset + 4);
        payload = Buffer.from(payload); // make writable copy
        for (let i = 0; i < payload.length; i++) {
          payload[i] ^= mask[i % 4];
        }
      }

      // Consume the frame from the buffer
      buffer = buffer.subarray(totalLen);

      // Handle by opcode
      if (opcode === 0x08) {
        // Close
        socket.end();
        return;
      }
      if (opcode === 0x09) {
        // Ping → Pong
        sendWsFrame(socket, 0x0a, payload);
        continue;
      }
      if (opcode === 0x01) {
        // Text frame
        const text = payload.toString('utf-8');
        handleSocketIOMessage(socket, text, mockSid);
      }
    }
  });

  socket.on('error', () => {});
  socket.on('close', () => {});
}

function handleSocketIOMessage(socket, text, sid) {
  // Engine.IO PING (type "2") → respond with PONG ("3")
  if (text === '2') {
    sendWsText(socket, '3');
    return;
  }

  // Socket.IO CONNECT (type "40") → respond with CONNECT ACK
  if (text.startsWith('40')) {
    sendWsText(socket, `40{"sid":"${sid}"}`);
    return;
  }

  // Socket.IO EVENT (type "42") → log but ignore
  // e.g. 42["tool:sync", {...}]
}

function sendWsText(socket, text) {
  sendWsFrame(socket, 0x01, Buffer.from(text, 'utf-8'));
}

function sendWsFrame(socket, opcode, payload) {
  if (socket.destroyed) return;

  const len = payload.length;
  let header;

  if (len < 126) {
    header = Buffer.alloc(2);
    header[0] = 0x80 | opcode; // FIN + opcode
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }

  try {
    socket.write(header);
    socket.write(payload);
  } catch {
    // socket may have been destroyed
  }
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

let server = null;

export function startMockServer(port = DEFAULT_PORT) {
  return new Promise((resolve, reject) => {
    server = http.createServer((req, res) => {
      handleRequest(req, res).catch(err => {
        console.error('[MockServer] Unhandled error:', err);
        json(res, 500, { success: false, error: 'Internal mock error' });
      });
    });

    // Handle WebSocket upgrades for Socket.IO
    server.on('upgrade', (req, socket, head) => {
      handleWebSocketUpgrade(req, socket, head);
    });

    server.on('error', reject);

    server.listen(port, '127.0.0.1', () => {
      console.log(`[MockServer] Listening on http://127.0.0.1:${port}`);
      resolve({ port });
    });
  });
}

export function stopMockServer() {
  return new Promise(resolve => {
    if (!server) {
      resolve();
      return;
    }
    server.close(() => {
      console.log('[MockServer] Stopped');
      server = null;
      resolve();
    });
  });
}
