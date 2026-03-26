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
// Mock behavior toggles — tests can change responses at runtime
// ---------------------------------------------------------------------------

let mockBehavior: Record<string, string> = {};

export function setMockBehavior(key: string, value: string) {
  mockBehavior[key] = value;
}

export function resetMockBehavior() {
  mockBehavior = {};
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
// Dynamic mock data helpers
// ---------------------------------------------------------------------------

/**
 * Build a team object whose subscription reflects the current mockBehavior.
 *   mockBehavior['plan']       → 'FREE' | 'BASIC' | 'PRO'  (default: 'FREE')
 *   mockBehavior['planActive'] → 'true' to mark subscription active
 *   mockBehavior['planExpiry'] → ISO date string for renewal display
 */
function getMockTeam() {
  const plan = mockBehavior['plan'] || 'FREE';
  const isActive = mockBehavior['planActive'] === 'true';
  const expiry = mockBehavior['planExpiry'] || null;

  return {
    team: {
      _id: 'team-1',
      name: 'Personal',
      slug: 'personal',
      createdBy: 'test-user-123',
      isPersonal: true,
      maxMembers: 1,
      subscription: { plan, hasActiveSubscription: isActive, planExpiry: expiry },
      usage: { dailyTokenLimit: 1000, remainingTokens: 1000, activeSessionCount: 0 },
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    },
    role: 'ADMIN',
  };
}

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
    if (mockBehavior['token'] === 'expired') {
      json(res, 401, { success: false, error: 'Token expired or invalid' });
      return;
    }
    if (mockBehavior['token'] === 'invalid') {
      json(res, 401, { success: false, error: 'Invalid token' });
      return;
    }
    const jwt = mockBehavior['jwt'] ? `${MOCK_JWT}-${mockBehavior['jwt']}` : MOCK_JWT;
    json(res, 200, { success: true, data: { jwtToken: jwt } });
    return;
  }

  // GET /telegram/me
  if (method === 'GET' && /^\/telegram\/me\/?(\?.*)?$/.test(url)) {
    if (mockBehavior['session'] === 'revoked') {
      json(res, 401, { success: false, error: 'Unauthorized' });
      return;
    }
    json(res, 200, { success: true, data: MOCK_USER });
    return;
  }

  // GET /teams
  if (method === 'GET' && /^\/teams\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [getMockTeam()] });
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

  // GET /billing/current-plan  (legacy alias)
  // GET /payments/stripe/currentPlan
  if (
    (method === 'GET' && /^\/billing\/current-plan\/?(\?.*)?$/.test(url)) ||
    (method === 'GET' && /^\/payments\/stripe\/currentPlan\/?(\?.*)?$/.test(url))
  ) {
    const plan = mockBehavior['plan'] || 'FREE';
    const isActive = mockBehavior['planActive'] === 'true';
    const expiry = mockBehavior['planExpiry'] || null;
    json(res, 200, {
      success: true,
      data: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
        subscription: isActive
          ? {
              id: 'sub_mock_123',
              status: 'active',
              currentPeriodEnd: expiry || new Date(Date.now() + 30 * 86400000).toISOString(),
            }
          : null,
      },
    });
    return;
  }

  // POST /payments/stripe/purchasePlan
  if (method === 'POST' && /^\/payments\/stripe\/purchasePlan\/?$/.test(url)) {
    if (mockBehavior['purchaseError'] === 'true') {
      json(res, 500, { success: false, error: 'Payment service unavailable' });
      return;
    }
    json(res, 200, {
      success: true,
      data: {
        checkoutUrl: 'http://127.0.0.1:18473/mock-checkout',
        sessionId: 'cs_mock_' + Date.now(),
      },
    });
    return;
  }

  // POST /payments/stripe/portal
  if (method === 'POST' && /^\/payments\/stripe\/portal\/?$/.test(url)) {
    json(res, 200, { success: true, data: { portalUrl: 'http://127.0.0.1:18473/mock-portal' } });
    return;
  }

  // POST /payments/coinbase/charge
  if (method === 'POST' && /^\/payments\/coinbase\/charge\/?$/.test(url)) {
    if (mockBehavior['coinbaseError'] === 'true') {
      json(res, 500, { success: false, error: 'Coinbase service unavailable' });
      return;
    }
    const chargeId = 'coinbase_mock_' + Date.now();
    json(res, 200, {
      success: true,
      data: {
        gatewayTransactionId: chargeId,
        hostedUrl: 'http://127.0.0.1:18473/mock-coinbase-checkout',
        status: 'NEW',
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return;
  }

  // GET /payments/coinbase/charge/:id — crypto payment status check
  if (method === 'GET' && /^\/payments\/coinbase\/charge\/[^/]+\/?(\?.*)?$/.test(url)) {
    const status = mockBehavior['cryptoStatus'] || 'NEW';
    const underpaidAmount = mockBehavior['cryptoUnderpaidAmount'] || '0';
    const overpaidAmount = mockBehavior['cryptoOverpaidAmount'] || '0';
    json(res, 200, {
      success: true,
      data: {
        status,
        payment: {
          status,
          amountPaid:
            status === 'UNDERPAID' ? '150.00' : status === 'OVERPAID' ? '350.00' : '250.00',
          amountExpected: '250.00',
          currency: 'USDC',
          underpaidAmount,
          overpaidAmount,
        },
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return;
  }

  // GET /auth/telegram/connect — OAuth connect URL for Telegram setup
  if (method === 'GET' && /^\/auth\/telegram\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior['telegramDuplicate'] === 'true') {
      json(res, 409, { success: false, error: 'Telegram account already linked to another user' });
      return;
    }
    json(res, 200, {
      success: true,
      data: { oauthUrl: 'http://127.0.0.1:18473/mock-telegram-oauth' },
    });
    return;
  }

  // POST /telegram/command — process a Telegram command
  if (method === 'POST' && /^\/telegram\/command\/?$/.test(url)) {
    if (mockBehavior['telegramUnauthorized'] === 'true') {
      json(res, 403, { success: false, error: 'Unauthorized: insufficient permissions' });
      return;
    }
    if (mockBehavior['telegramCommandError'] === 'true') {
      json(res, 400, { success: false, error: 'Invalid command format' });
      return;
    }
    json(res, 200, { success: true, data: { result: 'Command executed successfully' } });
    return;
  }

  // GET /telegram/permissions — get current permission levels
  if (method === 'GET' && /^\/telegram\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior['telegramPermission'] || 'read';
    json(res, 200, {
      success: true,
      data: { level, canRead: true, canWrite: level !== 'read', canInitiate: level === 'admin' },
    });
    return;
  }

  // POST /telegram/webhook/configure — configure webhook
  if (method === 'POST' && /^\/telegram\/webhook\/configure\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { webhookUrl: 'https://api.example.com/webhook/telegram', active: true },
    });
    return;
  }

  // POST /telegram/disconnect — disconnect Telegram skill
  if (method === 'POST' && /^\/telegram\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return;
  }

  // GET /skills — list available skills
  if (method === 'GET' && /^\/skills\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: [
        {
          id: 'telegram',
          name: 'Telegram',
          status: mockBehavior['telegramSkillStatus'] || 'installed',
          setupComplete: mockBehavior['telegramSetupComplete'] === 'true',
        },
        {
          id: 'notion',
          name: 'Notion',
          status: mockBehavior['notionSkillStatus'] || 'installed',
          setupComplete: mockBehavior['notionSetupComplete'] === 'true',
        },
        {
          id: 'email',
          name: 'Email',
          status: mockBehavior['gmailSkillStatus'] || 'installed',
          setupComplete: mockBehavior['gmailSetupComplete'] === 'true',
        },
      ],
    });
    return;
  }

  // GET /mock-telegram-oauth — mock OAuth page
  if (method === 'GET' && /^\/mock-telegram-oauth\/?(\?.*)?$/.test(url)) {
    setCors(res);
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end('<html><body><h1>Mock Telegram OAuth</h1></body></html>');
    return;
  }

  // GET /auth/notion/connect — OAuth connect URL for Notion setup
  if (method === 'GET' && /^\/auth\/notion\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior['notionTokenRevoked'] === 'true') {
      json(res, 401, { success: false, error: 'OAuth token has been revoked' });
      return;
    }
    const workspace = mockBehavior['notionWorkspace'] || "Test User's Workspace";
    json(res, 200, {
      success: true,
      data: { oauthUrl: 'http://127.0.0.1:18473/mock-notion-oauth', workspace },
    });
    return;
  }

  // GET /notion/permissions — get current Notion permission level
  if (method === 'GET' && /^\/notion\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior['notionPermission'] || 'read';
    json(res, 200, {
      success: true,
      data: {
        level,
        canRead: true,
        canWrite: level === 'write' || level === 'admin',
        canCreate: level === 'write' || level === 'admin',
      },
    });
    return;
  }

  // GET /mock-notion-oauth — mock Notion OAuth page
  if (method === 'GET' && /^\/mock-notion-oauth\/?(\?.*)?$/.test(url)) {
    setCors(res);
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end(
      '<html><body><h1>Mock Notion OAuth</h1><p>Authorize workspace access</p></body></html>'
    );
    return;
  }

  // GET /auth/google/connect — OAuth connect URL for Gmail/Email setup
  if (method === 'GET' && /^\/auth\/google\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior['gmailTokenRevoked'] === 'true') {
      json(res, 401, { success: false, error: 'OAuth token has been revoked' });
      return;
    }
    if (mockBehavior['gmailTokenExpired'] === 'true') {
      json(res, 401, { success: false, error: 'OAuth token has expired' });
      return;
    }
    json(res, 200, {
      success: true,
      data: { oauthUrl: 'http://127.0.0.1:18473/mock-google-oauth' },
    });
    return;
  }

  // GET /gmail/permissions — get current Gmail/Email permission level
  if (method === 'GET' && /^\/gmail\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior['gmailPermission'] || 'read';
    json(res, 200, {
      success: true,
      data: {
        level,
        canRead: true,
        canWrite: level === 'write' || level === 'admin',
        canInitiate: level === 'admin',
      },
    });
    return;
  }

  // POST /gmail/disconnect — disconnect Gmail/Email skill
  if (method === 'POST' && /^\/gmail\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return;
  }

  // GET /gmail/emails — fetch emails (mock list)
  if (method === 'GET' && /^\/gmail\/emails\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: [
        {
          id: 'msg-1',
          subject: 'Welcome to OpenHuman',
          from: 'team@alphahuman.com',
          date: new Date().toISOString(),
          snippet: 'Welcome to the platform!',
          hasAttachments: false,
        },
        {
          id: 'msg-2',
          subject: 'Weekly Crypto Report',
          from: 'reports@crypto.com',
          date: new Date(Date.now() - 86400000).toISOString(),
          snippet: 'Your weekly summary is ready.',
          hasAttachments: true,
        },
      ],
    });
    return;
  }

  // GET /mock-google-oauth — mock Google OAuth page
  if (method === 'GET' && /^\/mock-google-oauth\/?(\?.*)?$/.test(url)) {
    setCors(res);
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end('<html><body><h1>Mock Google OAuth</h1><p>Authorize email access</p></body></html>');
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
const openSockets = new Set();

export function startMockServer(port = DEFAULT_PORT) {
  return new Promise((resolve, reject) => {
    server = http.createServer((req, res) => {
      handleRequest(req, res).catch(err => {
        console.error('[MockServer] Unhandled error:', err);
        json(res, 500, { success: false, error: 'Internal mock error' });
      });
    });

    // Track all connections so stopMockServer can force-close them
    server.on('connection', socket => {
      openSockets.add(socket);
      socket.on('close', () => openSockets.delete(socket));
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
    // Destroy all open sockets so server.close() doesn't hang
    for (const socket of openSockets) {
      socket.destroy();
    }
    openSockets.clear();
    server.close(() => {
      console.log('[MockServer] Stopped');
      server = null;
      resolve();
    });
  });
}
