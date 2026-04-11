#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────────────
// debug-composio-trigger.mjs
//
// Composio trigger — Socket.IO live listener.
//
// Rust-side counterpart to
//   backend-1/src/scripts/live-test-composio-trigger.ts
//
// Opens a socket.io client against the openhuman backend (same endpoint
// the Rust core's SocketManager hits), authenticates with a JWT, and
// waits for `composio:trigger` events to land on the socket when the
// backend's POST /webhooks/composio receives and HMAC-verifies an
// incoming Composio webhook.
//
// End-to-end path under test:
//
//   [Gmail event]
//      └─► Composio fires webhook
//             └─► POST /webhooks/composio (HMAC verified)
//                    └─► handleWebhook.ts → emit('composio:trigger', …)
//                           └─► this script (or the Rust core) receives the event
//
// Prerequisites:
//   1. The backend is reachable at BACKEND_URL.
//   2. The backend is publicly addressable at the URL you configured in
//      Composio's dashboard for the webhook (usually via ngrok for local
//      dev). If Composio can't POST to /webhooks/composio, no events
//      will ever land on the socket — no amount of listening will help.
//   3. The test user already has an ACTIVE gmail connection — run
//      `bash scripts/debug-composio-login.sh` first to set one up.
//   4. A trigger instance exists for the user. Unlike the backend
//      script, this one does NOT create the trigger — we don't have
//      the Composio API key on the client. Create it once via the
//      backend team's `src/scripts/live-test-composio-trigger.ts`
//      (with CLEANUP=keep) or via the Composio dashboard, then run
//      this script as many times as you like.
//
// Usage:
//   node scripts/debug-composio-trigger.mjs
//   node scripts/debug-composio-trigger.mjs --timeout 600
//   node scripts/debug-composio-trigger.mjs --debug
//   node scripts/debug-composio-trigger.mjs --trigger GMAIL_NEW_GMAIL_MESSAGE
//   node scripts/debug-composio-trigger.mjs --max-events 3
//   node scripts/debug-composio-trigger.mjs --send-test   # (placeholder — see below)
//
// Env vars (loaded from .env + app/.env.local):
//   BACKEND_URL / VITE_BACKEND_URL — backend API base
//   JWT_TOKEN                       — bearer JWT (optional, overrides
//                                     the `openhuman-core auth get_session_token`
//                                     fallback)
//   TRIGGER_SLUG                    — override via CLI flag `--trigger`
// ──────────────────────────────────────────────────────────────────────

import { execSync } from 'child_process';
import { existsSync, readFileSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');

// ── CLI argument parsing ────────────────────────────────────────────

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const valueOf = (name, fallback) => {
  const idx = args.indexOf(name);
  if (idx === -1 || idx === args.length - 1) return fallback;
  return args[idx + 1];
};

const DEBUG = flag('--debug');
const TIMEOUT_SECS = parseInt(valueOf('--timeout', '0'), 10); // 0 = forever
const MAX_EVENTS = parseInt(valueOf('--max-events', '0'), 10); // 0 = unlimited
const TRIGGER_SLUG = (valueOf('--trigger', process.env.TRIGGER_SLUG || 'GMAIL_NEW_GMAIL_MESSAGE')).trim();

function dbg(...a) {
  if (DEBUG) console.log('  [debug]', ...a);
}

// ── Env loader (matches test-channel-receive.mjs) ───────────────────

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

// ── Pretty-print helpers ────────────────────────────────────────────

const C = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  cyan: '\x1b[36m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
};

function header(text) {
  console.log(`\n${C.cyan}${'─'.repeat(60)}${C.reset}`);
  console.log(`${C.cyan}  ${text}${C.reset}`);
  console.log(`${C.cyan}${'─'.repeat(60)}${C.reset}`);
}
function ok(detail) {
  console.log(`${C.green}  ✓${C.reset}${detail ? ` ${C.dim}${detail}${C.reset}` : ''}`);
}
function fail(detail) {
  console.log(`${C.red}  ✗ ${detail}${C.reset}`);
}
function info(label, value) {
  console.log(`${C.dim}    ${label}: ${C.reset}${value}`);
}
function ts() {
  return new Date().toISOString().replace('T', ' ').slice(0, 19);
}

// ── Banner ──────────────────────────────────────────────────────────

console.log('');
console.log(`${C.bold}📡 Composio Trigger — Socket.IO Live Listener${C.reset}`);
console.log('');
info('Backend', BACKEND_URL);
info('Trigger', TRIGGER_SLUG);
info('Timeout', TIMEOUT_SECS > 0 ? `${TIMEOUT_SECS}s` : 'forever (Ctrl+C to stop)');
info('Max events', MAX_EVENTS > 0 ? MAX_EVENTS : 'unlimited');
info('Debug', DEBUG);

// ── Resolve JWT ─────────────────────────────────────────────────────

header('1. Authentication');

function getSessionTokenFromCore() {
  const coreBin = path.join(ROOT, 'target', 'debug', 'openhuman-core');
  if (!existsSync(coreBin)) {
    if (DEBUG) console.debug(`[debug] core binary not found at ${coreBin}`);
    return null;
  }
  try {
    const output = execSync(`"${coreBin}" auth get_session_token`, {
      cwd: ROOT,
      encoding: 'utf-8',
      timeout: 10_000,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const match = output.match(/"token":\s*"([^"]+)"/);
    return match?.[1] || null;
  } catch (err) {
    if (DEBUG) {
      const msg = err instanceof Error ? err.message : String(err);
      console.debug(`[debug] ${coreBin} auth get_session_token failed: ${msg}`);
    }
    return null;
  }
}

let token = process.env.JWT_TOKEN?.trim() || null;
if (token) {
  info('Source', 'JWT_TOKEN env');
} else {
  token = getSessionTokenFromCore();
  if (token) info('Source', 'target/debug/openhuman-core auth get_session_token');
}

if (!token) {
  fail('No JWT_TOKEN in env and no session token available from the core binary.');
  console.log(
    `${C.dim}    Set JWT_TOKEN in .env, or run the app once to log in so the core has a session.${C.reset}`,
  );
  process.exit(1);
}
ok(`token=${token.slice(0, 20)}…`);

// ── Verify JWT against /auth/me ─────────────────────────────────────

header('2. Verify Token');

// We hold onto these so we can print them alongside every dropped-event
// diagnostic below. If the trigger was registered under a different
// user, the ids printed here will NOT match whatever the backend's
// `getSocketsByUserId(verified.payload.userId)` uses, and the emit is
// silently dropped.
let authedUserId = null;
let authedUserLabel = '(unknown)';

try {
  const resp = await fetch(`${BACKEND_URL}/auth/me`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const data = await resp.json();
  if (resp.ok && data.success && data.data) {
    const u = data.data;
    authedUserId = u._id || u.id || null;
    authedUserLabel = u.username || u.firstName || authedUserId || '(unknown)';
    ok(`user=${authedUserLabel}`);
    // Print the exact socket-map key the backend will use for this
    // connection. Compare against the trigger's registered userId if
    // you're seeing "backend captured, socket didn't".
    info('mongo _id', authedUserId ?? '(missing from /auth/me)');
    if (u.telegramId != null) info('telegramId', u.telegramId);
  } else {
    fail(`token invalid: ${JSON.stringify(data).slice(0, 300)}`);
    process.exit(1);
  }
} catch (err) {
  fail(`backend unreachable: ${err.message}`);
  process.exit(1);
}

// ── Verify gmail connection is ACTIVE ───────────────────────────────

header('3. Verify Gmail Connection');

try {
  const resp = await fetch(`${BACKEND_URL}/agent-integrations/composio/connections`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const data = await resp.json();
  if (!resp.ok || !data.success) {
    fail(`GET /connections → HTTP ${resp.status}: ${data?.error || 'unknown error'}`);
    process.exit(1);
  }
  const connections = data.data?.connections ?? [];
  const gmail = connections.find((c) => c.toolkit?.toLowerCase() === 'gmail');
  if (!gmail) {
    fail('no gmail connection found for this user');
    console.log(
      `${C.yellow}    Run: bash scripts/debug-composio-login.sh first to connect gmail.${C.reset}`,
    );
    process.exit(1);
  }
  const status = String(gmail.status).toUpperCase();
  if (status === 'ACTIVE' || status === 'CONNECTED') {
    ok(`gmail ACTIVE (id=${gmail.id})`);
  } else {
    console.log(
      `${C.yellow}  ⚠ gmail connection status is "${gmail.status}" — trigger may not fire.${C.reset}`,
    );
    info('connection id', gmail.id);
  }
} catch (err) {
  fail(`connection check failed: ${err.message}`);
  process.exit(1);
}

// ── Trigger-create reminder ─────────────────────────────────────────
//
// We deliberately don't create the Composio trigger from this script:
// the Composio SDK needs COMPOSIO_API_KEY, which is a backend secret.
// The backend team's live-test-composio-trigger.ts already handles
// trigger creation — run it once with CLEANUP=keep and then this
// listener will see every fired event until you explicitly delete
// the trigger.

header('4. Trigger Setup Reminder');
console.log(
  `${C.dim}  This script only LISTENS. It does not create/delete${C.reset}\n` +
    `${C.dim}  Composio triggers (that requires the backend's COMPOSIO_API_KEY).${C.reset}\n\n` +
    `${C.dim}  To create the trigger once, run the backend team's tool:${C.reset}\n` +
    `${C.dim}    CLEANUP=keep TRIGGER_SLUG=${TRIGGER_SLUG} \\${C.reset}\n` +
    `${C.dim}      npx ts-node -r tsconfig-paths/register \\${C.reset}\n` +
    `${C.dim}      src/scripts/live-test-composio-trigger.ts${C.reset}\n\n` +
    `${C.dim}  Then run THIS script from the openhuman-2 repo to verify${C.reset}\n` +
    `${C.dim}  that trigger events reach the client socket.${C.reset}`,
);

// ── Load socket.io-client ───────────────────────────────────────────

header('5. Connect Socket.IO');

const socketIoPath = path.join(
  ROOT,
  'app',
  'node_modules',
  'socket.io-client',
  'dist',
  'socket.io.esm.min.js',
);

let io;
try {
  // Prefer the version co-located with the app workspace — same binary
  // the React client uses at runtime.
  if (existsSync(socketIoPath)) {
    const mod = await import(socketIoPath);
    io = mod.io || mod.default;
    dbg('loaded socket.io-client from', socketIoPath);
  } else {
    const mod = await import('socket.io-client');
    io = mod.io || mod.default;
    dbg('loaded socket.io-client from node resolution');
  }
} catch (err) {
  fail(`cannot load socket.io-client: ${err.message}`);
  console.log(
    `${C.dim}    Try: cd app && yarn install${C.reset}`,
  );
  process.exit(1);
}

const socket = io(BACKEND_URL, {
  auth: { token },
  transports: ['websocket', 'polling'],
  path: '/socket.io/',
  reconnection: true,
  reconnectionAttempts: 5,
  timeout: 10_000,
});

// ── Catchall: log every event the server sends us ─────────────────
//
// This is on regardless of --debug because the #1 reason "the backend
// logged a webhook but my socket didn't" is that the emit targeted a
// different userId. If this catchall is silent during the wait, your
// socket is NOT receiving any server traffic at all — either because
// the server thinks the socket is dead, or because your auth maps to
// a user the backend isn't emitting to.
//
// `onAny` is the socket.io v4 blessed API for catchall listeners.
// Normal named `.on('composio:trigger', …)` handlers still fire after
// this runs.
socket.onAny((eventName, ...payload) => {
  const first = payload[0];
  const preview =
    first === undefined
      ? ''
      : ` ${JSON.stringify(first).slice(0, 160)}`;
  console.log(
    `${C.dim}  [${ts()}] ⇢ ${eventName}${preview}${C.reset}`,
  );
});

// Low-frequency heartbeat so silent disconnects are obvious. If the
// transport dies without firing `disconnect` on the client, this will
// let us notice by the absence of any traffic over 30s.
let lastEventAt = Date.now();
socket.onAny(() => {
  lastEventAt = Date.now();
});
const heartbeatTimer = setInterval(() => {
  const idleSecs = Math.round((Date.now() - lastEventAt) / 1000);
  if (idleSecs >= 30 && socket.connected) {
    console.log(
      `${C.dim}  [${ts()}] idle ${idleSecs}s (socket.connected=${socket.connected}, transport=${socket.io.engine?.transport?.name})${C.reset}`,
    );
  }
}, 15_000);

// ── Connection lifecycle ────────────────────────────────────────────

let eventCount = 0;

socket.on('connect', () => {
  ok(`connected (socket.id=${socket.id})`);
  dbg('transport:', socket.io.engine?.transport?.name);
});

socket.on('ready', () => {
  console.log(`${C.green}  🟢 server ready — auth accepted${C.reset}`);
  header('6. Listening for composio:trigger');
  console.log(
    `${C.magenta}  Take the action that fires ${TRIGGER_SLUG}${C.reset}`,
  );
  console.log(
    `${C.dim}  (e.g. for GMAIL_NEW_GMAIL_MESSAGE, send yourself an email).${C.reset}`,
  );
  console.log(`${C.dim}  Press Ctrl+C to stop.${C.reset}\n`);
});

socket.on('connect_error', (err) => {
  fail(`connect_error: ${err.message}`);
  dbg('full error:', err);
});

socket.on('error', (data) => {
  fail(`server error: ${JSON.stringify(data)}`);
});

socket.on('disconnect', (reason) => {
  console.log(`${C.yellow}  🔴 disconnected: ${reason}${C.reset}`);
});

socket.io.on('reconnect', (attempt) => {
  console.log(`${C.yellow}  🔄 reconnected after ${attempt} attempt(s)${C.reset}`);
});

// ── The event under test ────────────────────────────────────────────

function printEvent(event) {
  eventCount++;
  const label = event?.trigger || '(unknown)';
  const toolkit = event?.toolkit || '(unknown)';
  console.log(
    `${C.green}  [${ts()}] #${eventCount} composio:trigger${C.reset} ` +
      `${C.bold}${label}${C.reset} ${C.dim}(${toolkit})${C.reset}`,
  );
  if (event?.metadata) {
    info('metadata', JSON.stringify(event.metadata));
  }
  const payload = event?.payload ?? {};
  const preview = JSON.stringify(payload, null, 2) || '{}';
  const lines = preview.split('\n');
  const limited = lines.slice(0, 20).join('\n');
  console.log(
    `${C.dim}    payload:\n${limited
      .split('\n')
      .map((l) => `      ${l}`)
      .join('\n')}${C.reset}`,
  );
  if (lines.length > 20) {
    console.log(`${C.dim}      …(${lines.length - 20} more lines)${C.reset}`);
  }
  console.log('');
}

const done = new Promise((resolve) => {
  socket.on('composio:trigger', (event) => {
    try {
      // Filter on slug when the caller passed --trigger so you can
      // keep a broad listener running while debugging one hook at a
      // time. `event.trigger` is the slug emitted by the backend.
      if (
        TRIGGER_SLUG &&
        event?.trigger &&
        event.trigger.toUpperCase() !== TRIGGER_SLUG.toUpperCase()
      ) {
        dbg(`ignoring ${event.trigger} (filter=${TRIGGER_SLUG})`);
      } else {
        printEvent(event);
      }
    } catch (err) {
      fail(`error printing event: ${err.message}`);
    }
    if (MAX_EVENTS > 0 && eventCount >= MAX_EVENTS) {
      console.log(
        `${C.yellow}  Reached --max-events=${MAX_EVENTS} — shutting down.${C.reset}`,
      );
      resolve();
    }
  });

  // Optional hard timeout.
  if (TIMEOUT_SECS > 0) {
    setTimeout(() => {
      console.log(
        `${C.yellow}  --timeout=${TIMEOUT_SECS}s elapsed — shutting down.${C.reset}`,
      );
      resolve();
    }, TIMEOUT_SECS * 1_000);
  }

  // Ctrl+C.
  process.on('SIGINT', () => {
    console.log(`\n${C.yellow}  SIGINT received — shutting down.${C.reset}`);
    resolve();
  });
});

await done;

// ── Cleanup ─────────────────────────────────────────────────────────

header('7. Cleanup');
clearInterval(heartbeatTimer);
try {
  socket.close();
  ok('socket closed');
} catch {
  fail('failed to close socket');
}

if (eventCount === 0) {
  // Surface the three usual suspects. This is the script talking, not
  // the backend — the backend drops mismatched emits silently, so it
  // won't tell us which one we're hitting.
  console.log('');
  console.log(
    `${C.yellow}${C.bold}  No composio:trigger events received.${C.reset}`,
  );
  console.log('');
  console.log(`${C.dim}  Checklist:${C.reset}`);
  console.log(
    `${C.dim}  1. Trigger userId vs your user:${C.reset}\n` +
      `${C.dim}     Your socket is authed as _id=${C.reset}${C.bold}${authedUserId ?? '(unknown)'}${C.reset}${C.dim}.${C.reset}\n` +
      `${C.dim}     The trigger must have been CREATED with that exact userId on${C.reset}\n` +
      `${C.dim}     the Composio side. If you used the backend's${C.reset}\n` +
      `${C.dim}     live-test-composio-trigger.ts, it registers with user.id${C.reset}\n` +
      `${C.dim}     from whichever TELEGRAM_ID/USER_ID/JWT_TOKEN you ran it with.${C.reset}\n` +
      `${C.dim}     If those don't match, the backend's getSocketsByUserId${C.reset}\n` +
      `${C.dim}     returns [] and the emit is dropped silently.${C.reset}`,
  );
  console.log('');
  console.log(
    `${C.dim}  2. Did the catchall above log ANY inbound events?${C.reset}\n` +
      `${C.dim}     - If yes (e.g. "ready", "toast", heartbeat), your socket is${C.reset}\n` +
      `${C.dim}       alive and the issue is targeting (#1).${C.reset}\n` +
      `${C.dim}     - If no, the backend thinks your socket is gone. Check${C.reset}\n` +
      `${C.dim}       whether another client (the Rust core running locally?)${C.reset}\n` +
      `${C.dim}       is holding the slot, or that your JWT hasn't expired${C.reset}\n` +
      `${C.dim}       mid-run.${C.reset}`,
  );
  console.log('');
  console.log(
    `${C.dim}  3. Add one-line logging to the backend's${C.reset}\n` +
      `${C.dim}     src/controllers/agentIntegrations/composio/handleWebhook.ts${C.reset}\n` +
      `${C.dim}     right after the getSocketsByUserId call so the backend tells${C.reset}\n` +
      `${C.dim}     you how many sockets it matched for which userId. That is${C.reset}\n` +
      `${C.dim}     the only truly authoritative signal — everything else is a${C.reset}\n` +
      `${C.dim}     guess.${C.reset}`,
  );
  console.log('');
  process.exit(2);
}

console.log(
  `\n${C.green}${C.bold}  Received ${eventCount} composio:trigger event(s).${C.reset}\n`,
);
process.exit(0);
