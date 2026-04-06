#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────────────
// test-channel-receive.mjs
//
// Connects to the backend Socket.IO server, authenticates with the
// stored session JWT, and listens for incoming channel messages.
//
// Usage:
//   node scripts/test-channel-receive.mjs
//   node scripts/test-channel-receive.mjs --timeout 120
//   node scripts/test-channel-receive.mjs --debug          # verbose logging
//   node scripts/test-channel-receive.mjs --send-test      # also send a test msg
// ──────────────────────────────────────────────────────────────────────

import { execSync } from 'child_process';
import { existsSync, readFileSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');

const args = process.argv.slice(2);
const DEBUG = args.includes('--debug');
const SEND_TEST = args.includes('--send-test');

// ── Load env ────────────────────────────────────────────────────────
function loadEnv(filepath) {
  if (!existsSync(filepath)) return;
  const lines = readFileSync(filepath, 'utf-8').split('\n');
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const eqIdx = trimmed.indexOf('=');
    if (eqIdx < 0) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    const val = trimmed.slice(eqIdx + 1).trim();
    if (!process.env[key]) process.env[key] = val;
  }
}

loadEnv(path.join(ROOT, '.env'));
loadEnv(path.join(ROOT, 'app', '.env.local'));

const BACKEND_URL = (
  process.env.BACKEND_URL ||
  process.env.VITE_BACKEND_URL ||
  'https://staging-api.alphahuman.xyz'
).replace(/\/+$/, '');

const TIMEOUT_SECS = parseInt(args.find((_, i, a) => a[i - 1] === '--timeout') || '60', 10);

function dbg(...args) {
  if (DEBUG) console.log('  [debug]', ...args);
}

// ── Get session token from core ─────────────────────────────────────
function getSessionToken() {
  const coreBin = path.join(ROOT, 'target', 'debug', 'openhuman-core');
  try {
    const output = execSync(`"${coreBin}" auth get_session_token`, {
      cwd: ROOT,
      encoding: 'utf-8',
      timeout: 10_000,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const match = output.match(/"token":\s*"([^"]+)"/);
    return match?.[1] || null;
  } catch {
    return null;
  }
}

console.log('');
console.log('📡 Channel Receive Listener');
console.log('────────────────────────────────────────────────');
console.log(`Backend:   ${BACKEND_URL}`);
console.log(`Timeout:   ${TIMEOUT_SECS}s`);
console.log(`Debug:     ${DEBUG}`);
console.log(`Send test: ${SEND_TEST}`);
console.log('');

// ── Resolve token ───────────────────────────────────────────────────
console.log('🔑 Resolving session token...');
let token = getSessionToken();
if (!token) {
  console.error('   ❌ No session token found. Login via the app first.');
  process.exit(1);
}
console.log(`   ✅ Token: ${token.slice(0, 20)}...`);

// ── Validate token against backend ──────────────────────────────────
console.log('');
console.log('🔍 Validating token against backend...');
try {
  const resp = await fetch(`${BACKEND_URL}/auth/me`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const data = await resp.json();
  if (data.success && data.data) {
    const u = data.data;
    console.log(`   ✅ User: ${u.username || u.firstName || u._id}`);
    console.log(`   ✅ Telegram ID: ${u.telegramId || 'NOT LINKED'}`);
    if (!u.telegramId) {
      console.log('   ⚠️  No Telegram linked — incoming messages won\'t route to you');
    }
  } else {
    console.log(`   ❌ Token invalid: ${JSON.stringify(data)}`);
    process.exit(1);
  }
} catch (err) {
  console.log(`   ❌ Backend unreachable: ${err.message}`);
  process.exit(1);
}

// ── Connect Socket.IO ───────────────────────────────────────────────
const socketIoPath = path.join(ROOT, 'node_modules', 'socket.io-client', 'dist', 'socket.io.esm.min.js');
let io;
try {
  const mod = await import(socketIoPath);
  io = mod.io || mod.default;
} catch {
  try {
    const mod = await import('socket.io-client');
    io = mod.io || mod.default;
  } catch (err) {
    console.error('   ❌ Cannot load socket.io-client:', err.message);
    process.exit(1);
  }
}

console.log('');
console.log('🔌 Connecting to Socket.IO...');
dbg('URL:', BACKEND_URL);
dbg('Transport: websocket + polling');
dbg('Path: /socket.io/');

const socket = io(BACKEND_URL, {
  auth: { token },
  transports: ['websocket', 'polling'],
  path: '/socket.io/',
  reconnection: true,
  reconnectionAttempts: 3,
  timeout: 10_000,
});

let messageCount = 0;

// ── In debug mode, log ALL events ───────────────────────────────────
if (DEBUG) {
  const origOnevent = socket.onevent;
  socket.onevent = function (packet) {
    const eventName = packet.data?.[0];
    const eventData = packet.data?.slice(1);
    console.log(`  [debug] EVENT: ${eventName}`, JSON.stringify(eventData).slice(0, 200));
    origOnevent.call(this, packet);
  };
}

socket.on('connect', () => {
  console.log(`   ✅ Connected (socket id: ${socket.id})`);
  dbg('Transport:', socket.io.engine?.transport?.name);
  console.log('');
  console.log('────────────────────────────────────────────────');
  console.log('👂 Listening for incoming channel messages...');
  console.log('   Send a message to the Telegram bot now!');
  console.log(`   (auto-exit after ${TIMEOUT_SECS}s, or Ctrl+C)`);
  console.log('────────────────────────────────────────────────');
  console.log('');

  // If --send-test, fire off a test message after connecting
  if (SEND_TEST) {
    setTimeout(async () => {
      console.log('📤 Sending test message via backend REST API...');
      try {
        const resp = await fetch(`${BACKEND_URL}/channels/telegram/messages`, {
          method: 'POST',
          headers: {
            Authorization: `Bearer ${token}`,
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({ text: '🧪 Round-trip test from receive listener script' }),
        });
        const data = await resp.json();
        console.log(`   Response: ${JSON.stringify(data)}`);
        console.log('');
      } catch (err) {
        console.log(`   ❌ Send failed: ${err.message}`);
      }
    }, 1000);
  }
});

socket.on('ready', () => {
  console.log('   🟢 Server ready');
  dbg('Socket authenticated and registered on server');
});

socket.on('connect_error', (err) => {
  console.error(`   ❌ Connection error: ${err.message}`);
  dbg('Full error:', err);
});

socket.on('error', (data) => {
  console.error(`   ❌ Server error: ${JSON.stringify(data)}`);
});

socket.on('disconnect', (reason) => {
  console.log(`   🔴 Disconnected: ${reason}`);
});

socket.io.on('reconnect_attempt', (attempt) => {
  dbg(`Reconnect attempt #${attempt}`);
});

socket.io.on('reconnect', (attempt) => {
  console.log(`   🔄 Reconnected after ${attempt} attempt(s)`);
});

// ── Channel message events ──────────────────────────────────────────

// Inbound: Telegram user → bot → backend → socket → here
socket.on('channel:message', (data) => {
  messageCount++;
  const ts = new Date().toLocaleTimeString();
  console.log(`┌─ 📨 INBOUND #${messageCount} [${ts}]`);
  console.log(`│  Channel:  ${data.channel || 'unknown'}`);
  console.log(`│  UserId:   ${data.userId || 'unknown'}`);
  console.log(`│  Message:  ${data.message || '(empty)'}`);
  console.log(`│  Time:     ${data.receivedAt || 'unknown'}`);
  console.log(`└──────────────────────────────────`);
  console.log('');
  dbg('Full payload:', JSON.stringify(data));
});

// Outbound confirmation: app sent message → backend → Telegram, socket notified
socket.on('channel:message:sent', (data) => {
  const ts = new Date().toLocaleTimeString();
  console.log(`┌─ 📤 OUTBOUND CONFIRMED [${ts}]`);
  console.log(`│  Channel:  ${data.channel || 'unknown'}`);
  console.log(`│  Success:  ${data.result?.success}`);
  console.log(`│  MsgId:    ${data.result?.messageId || 'n/a'}`);
  if (data.message?.text) {
    console.log(`│  Text:     ${data.message.text.slice(0, 80)}`);
  }
  console.log(`└──────────────────────────────────`);
  console.log('');
  dbg('Full payload:', JSON.stringify(data));
});

// ── Timeout ─────────────────────────────────────────────────────────
const timer = setTimeout(() => {
  console.log('');
  console.log('────────────────────────────────────────────────');
  console.log(`⏰ Timeout (${TIMEOUT_SECS}s). Received ${messageCount} inbound message(s).`);
  socket.disconnect();
  process.exit(0);
}, TIMEOUT_SECS * 1000);

process.on('SIGINT', () => {
  clearTimeout(timer);
  console.log('');
  console.log('────────────────────────────────────────────────');
  console.log(`👋 Stopped. Received ${messageCount} inbound message(s).`);
  socket.disconnect();
  process.exit(0);
});
