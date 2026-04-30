#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────────────
// test-onboarding-chat.mjs
//
// Interactive test harness for the welcome (onboarding) agent.
// Resets `chat_onboarding_completed` in config, connects to the core
// server via Socket.IO, fires the onboarding trigger, and lets you
// chat back and forth with the welcome agent in your terminal.
//
// Prerequisites:
//   - Core server running: `pnpm dev` or `openhuman run`
//   - Logged in via the desktop app (session token required)
//
// Usage:
//   node scripts/test-onboarding-chat.mjs
//   node scripts/test-onboarding-chat.mjs --debug        # verbose event logging
//   node scripts/test-onboarding-chat.mjs --no-reset     # skip config reset
//   node scripts/test-onboarding-chat.mjs --no-trigger   # skip auto-trigger, type first msg yourself
// ──────────────────────────────────────────────────────────────────────

import { createInterface } from 'readline';
import { existsSync, readFileSync, writeFileSync, readdirSync } from 'fs';
import { homedir } from 'os';
import path from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');

// ── Args ───────────────────────────────────────────────────────────
const args = process.argv.slice(2);
const DEBUG = args.includes('--debug');
const NO_RESET = args.includes('--no-reset');
const NO_TRIGGER = args.includes('--no-trigger');

// ── Config ─────────────────────────────────────────────────────────
const CORE_PORT = process.env.OPENHUMAN_CORE_PORT || '7788';
const CORE_HOST = process.env.OPENHUMAN_CORE_HOST || '127.0.0.1';
const CORE_URL = `http://${CORE_HOST}:${CORE_PORT}`;

const OPENHUMAN_HOME = process.env.OPENHUMAN_WORKSPACE
  ? path.join(process.env.OPENHUMAN_WORKSPACE)
  : path.join(homedir(), '.openhuman');

// Set OPENHUMAN_USER_ID to pin to a specific user directory deterministically.
function findConfigPath() {
  const usersDir = path.join(OPENHUMAN_HOME, 'users');
  const pinnedId = process.env.OPENHUMAN_USER_ID;
  if (pinnedId) {
    const candidate = path.join(usersDir, pinnedId, 'config.toml');
    if (existsSync(candidate)) return candidate;
  }
  if (existsSync(usersDir)) {
    try {
      const entries = readdirSync(usersDir).filter(e => e !== 'local').sort();
      for (const entry of entries) {
        const candidate = path.join(usersDir, entry, 'config.toml');
        if (existsSync(candidate)) return candidate;
      }
    } catch { /* fall through */ }
  }
  return path.join(OPENHUMAN_HOME, 'config.toml');
}
const CONFIG_PATH = findConfigPath();

const TRIGGER_MESSAGE =
  'the user just finished the desktop onboarding wizard. welcome the user.';

const THREAD_ID = `test-onboarding-${randomUUID().slice(0, 8)}`;

// ── Helpers ────────────────────────────────────────────────────────
function dbg(...a) {
  if (DEBUG) console.log('\x1b[90m  [debug]\x1b[0m', ...a);
}

function log(msg) {
  console.log(`\x1b[36m[test]\x1b[0m ${msg}`);
}

function warn(msg) {
  console.log(`\x1b[33m[warn]\x1b[0m ${msg}`);
}

function err(msg) {
  console.error(`\x1b[31m[error]\x1b[0m ${msg}`);
}

// ── Reset config ───────────────────────────────────────────────────
function resetOnboardingConfig() {
  if (!existsSync(CONFIG_PATH)) {
    warn(`Config not found at ${CONFIG_PATH} — skipping reset`);
    return false;
  }

  let content = readFileSync(CONFIG_PATH, 'utf-8');
  const original = content;

  // Set chat_onboarding_completed = false
  if (content.includes('chat_onboarding_completed')) {
    content = content.replace(
      /chat_onboarding_completed\s*=\s*true/,
      'chat_onboarding_completed = false'
    );
  } else {
    // Add it if missing
    content = `chat_onboarding_completed = false\n${content}`;
  }

  if (content !== original) {
    writeFileSync(CONFIG_PATH, content, 'utf-8');
    log('Reset chat_onboarding_completed = false in config.toml');
    return true;
  } else {
    log('chat_onboarding_completed already false (or missing)');
    return false;
  }
}

// ── Check core server is running ───────────────────────────────────
async function checkCoreHealth() {
  try {
    const resp = await fetch(`${CORE_URL}/health`, { signal: AbortSignal.timeout(3000) });
    return resp.ok;
  } catch {
    return false;
  }
}

// ── Load socket.io-client ──────────────────────────────────────────
async function loadSocketIo() {
  // pnpm hoists into .pnpm — use createRequire from the app/ workspace
  // where socket.io-client is an actual dependency.
  const { createRequire } = await import('module');
  const dirs = [
    path.join(ROOT, 'app'),  // app workspace (has the dep)
    ROOT,                     // repo root fallback
  ];

  for (const dir of dirs) {
    try {
      const require = createRequire(path.join(dir, 'package.json'));
      const mod = require('socket.io-client');
      dbg('Loaded socket.io-client via createRequire from:', dir);
      return mod.io || mod.default || mod;
    } catch { /* fall through */ }
  }

  err('Cannot load socket.io-client. Run: cd app && pnpm install');
  process.exit(1);
}

// ── Main ───────────────────────────────────────────────────────────
async function main() {
  console.log('');
  console.log('\x1b[1m  Welcome Agent Test Harness\x1b[0m');
  console.log('  ─────────────────────────────────────');
  console.log(`  Core:      ${CORE_URL}`);
  console.log(`  Config:    ${CONFIG_PATH}`);
  console.log(`  Thread:    ${THREAD_ID}`);
  console.log(`  Debug:     ${DEBUG}`);
  console.log(`  Reset:     ${!NO_RESET}`);
  console.log(`  Trigger:   ${!NO_TRIGGER}`);
  console.log('');

  // Check core is running
  log('Checking core server health...');
  const healthy = await checkCoreHealth();
  if (!healthy) {
    err(`Core server not reachable at ${CORE_URL}`);
    err('Start it with: pnpm dev  (or: openhuman run)');
    process.exit(1);
  }
  log('Core server is up');

  // Reset onboarding
  if (!NO_RESET) {
    resetOnboardingConfig();
    // Give the core a moment to pick up the config change
    await new Promise((r) => setTimeout(r, 500));
  }

  // Load socket.io
  const io = await loadSocketIo();

  // Connect
  log('Connecting to core Socket.IO...');
  const socket = io(CORE_URL, {
    transports: ['websocket'],
    reconnection: false,
    timeout: 10_000,
  });

  let clientId = null;
  let responseBuffer = '';
  let isStreaming = false;
  let toolCalls = [];

  // ── Event handlers ─────────────────────────────────────────────
  socket.on('connect', () => {
    dbg('Socket connected, sid:', socket.id);
  });

  socket.on('ready', (data) => {
    clientId = data.sid;
    log(`Connected as client: ${clientId}`);
    console.log('');

    if (!NO_TRIGGER) {
      sendMessage(TRIGGER_MESSAGE, true);
    } else {
      promptUser();
    }
  });

  socket.on('connect_error', (e) => {
    err(`Connection failed: ${e.message}`);
    process.exit(1);
  });

  // Stream text deltas
  socket.on('text_delta', (data) => {
    dbg('text_delta:', JSON.stringify(data).slice(0, 200));
    if (data.delta) {
      if (!isStreaming) {
        isStreaming = true;
        process.stdout.write('\x1b[32m  '); // green for agent
      }
      process.stdout.write(data.delta);
      responseBuffer += data.delta;
    }
  });

  // Thinking deltas (reasoning model)
  socket.on('thinking_delta', (data) => {
    dbg('thinking_delta:', JSON.stringify(data).slice(0, 200));
    if (data.delta) {
      if (!isStreaming) {
        isStreaming = true;
        process.stdout.write('\x1b[90m  [thinking] ');
      }
      process.stdout.write(data.delta);
    }
  });

  // Inference start
  socket.on('inference_start', (data) => {
    dbg('inference_start:', JSON.stringify(data).slice(0, 200));
  });

  // Iteration start
  socket.on('iteration_start', (data) => {
    dbg('iteration_start:', JSON.stringify(data).slice(0, 200));
  });

  // Tool calls
  socket.on('tool_call', (data) => {
    if (isStreaming) {
      process.stdout.write('\x1b[0m\n');
      isStreaming = false;
    }
    const toolInfo = `${data.tool_name || 'unknown'}`;
    console.log(`\x1b[90m  [tool] ${toolInfo}\x1b[0m`);
    if (data.args && DEBUG) {
      console.log(`\x1b[90m         args: ${JSON.stringify(data.args).slice(0, 300)}\x1b[0m`);
    }
    toolCalls.push(toolInfo);
  });

  // Tool results
  socket.on('tool_result', (data) => {
    dbg('tool_result:', data.tool_name, data.success ? 'ok' : 'FAIL');
    if (DEBUG && data.output) {
      const preview = data.output.length > 500
        ? data.output.slice(0, 500) + '...'
        : data.output;
      console.log(`\x1b[90m  [tool_result] ${preview}\x1b[0m`);
    }
  });

  // Chat segments (multi-bubble)
  socket.on('chat_segment', (data) => {
    dbg('chat_segment:', data.segment_index, '/', data.segment_total);
    if (data.message) {
      if (!isStreaming) {
        process.stdout.write('\x1b[32m  ');
        isStreaming = true;
      }
      process.stdout.write(data.message);
      responseBuffer += data.message;
    }
  });

  // Chat done
  socket.on('chat_done', (data) => {
    if (isStreaming) {
      process.stdout.write('\x1b[0m\n');
      isStreaming = false;
    }

    if (data.full_response && !responseBuffer) {
      // Didn't get streamed, show full response
      console.log(`\x1b[32m  ${data.full_response}\x1b[0m`);
    }

    console.log('');
    if (toolCalls.length > 0) {
      console.log(`\x1b[90m  tools used: ${toolCalls.join(', ')}\x1b[0m`);
    }
    if (data.reaction_emoji) {
      console.log(`\x1b[90m  reaction: ${data.reaction_emoji}\x1b[0m`);
    }
    console.log('');

    responseBuffer = '';
    toolCalls = [];
    promptUser();
  });

  // Chat error
  socket.on('chat_error', (data) => {
    if (isStreaming) {
      process.stdout.write('\x1b[0m\n');
      isStreaming = false;
    }
    err(`Chat error (${data.error_type || 'unknown'}): ${data.message || 'no message'}`);
    console.log('');
    responseBuffer = '';
    toolCalls = [];
    promptUser();
  });

  // Debug: log all events
  if (DEBUG) {
    const origOnevent = socket.onevent;
    socket.onevent = function (packet) {
      const eventName = packet.data?.[0];
      if (!['text_delta', 'thinking_delta', 'tool_args_delta'].includes(eventName)) {
        console.log(`\x1b[90m  [event] ${eventName}: ${JSON.stringify(packet.data?.slice(1)).slice(0, 300)}\x1b[0m`);
      }
      origOnevent.call(this, packet);
    };
  }

  // ── Send message ───────────────────────────────────────────────
  function sendMessage(message, isTrigger = false) {
    if (!clientId) {
      warn('Not connected yet');
      return;
    }

    if (!isTrigger) {
      console.log('');
    }

    if (isTrigger) {
      log('Sending onboarding trigger...');
    } else {
      dbg('Sending:', message.slice(0, 100));
    }

    socket.emit('chat:start', {
      thread_id: THREAD_ID,
      message,
    });
  }

  // ── Interactive prompt ─────────────────────────────────────────
  const rl = createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  function promptUser() {
    rl.question('\x1b[1myou>\x1b[0m ', (input) => {
      const trimmed = input.trim();
      if (!trimmed) {
        promptUser();
        return;
      }
      if (trimmed === '/quit' || trimmed === '/exit' || trimmed === '/q') {
        log('Bye!');
        socket.disconnect();
        rl.close();
        process.exit(0);
      }
      if (trimmed === '/status') {
        log(`Thread: ${THREAD_ID}`);
        log(`Client: ${clientId}`);
        log(`Config: ${CONFIG_PATH}`);
        promptUser();
        return;
      }
      if (trimmed === '/reset') {
        resetOnboardingConfig();
        log('Config reset. Send a message to re-trigger welcome agent.');
        promptUser();
        return;
      }
      if (trimmed === '/trigger') {
        sendMessage(TRIGGER_MESSAGE, true);
        return;
      }
      if (trimmed === '/help') {
        console.log('');
        console.log('  Commands:');
        console.log('    /quit, /exit, /q   Exit');
        console.log('    /status            Show connection info');
        console.log('    /reset             Reset chat_onboarding_completed to false');
        console.log('    /trigger           Re-send the onboarding trigger message');
        console.log('    /help              This message');
        console.log('');
        promptUser();
        return;
      }
      sendMessage(trimmed);
    });
  }

  // Clean exit
  rl.on('close', () => {
    socket.disconnect();
    process.exit(0);
  });

  process.on('SIGINT', () => {
    console.log('');
    log('Interrupted');
    socket.disconnect();
    rl.close();
    process.exit(0);
  });
}

main().catch((e) => {
  err(e.message);
  process.exit(1);
});
