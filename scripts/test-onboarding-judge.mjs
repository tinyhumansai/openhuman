#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────────────
// test-onboarding-judge.mjs
//
// Non-interactive automated test for the welcome agent. Sends a sequence
// of scripted user messages, collects the agent's responses, and prints
// a judgment report at the end.
//
// Usage:
//   node scripts/test-onboarding-judge.mjs
//   node scripts/test-onboarding-judge.mjs --debug
// ──────────────────────────────────────────────────────────────────────

import { existsSync, readFileSync, writeFileSync } from 'fs';
import { homedir } from 'os';
import path from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');

const args = process.argv.slice(2);
const DEBUG = args.includes('--debug');

const CORE_PORT = process.env.OPENHUMAN_CORE_PORT || '7788';
const CORE_HOST = process.env.OPENHUMAN_CORE_HOST || '127.0.0.1';
const CORE_URL = `http://${CORE_HOST}:${CORE_PORT}`;

import { readdirSync } from 'fs';

const OPENHUMAN_HOME = process.env.OPENHUMAN_WORKSPACE
  ? path.join(process.env.OPENHUMAN_WORKSPACE)
  : path.join(homedir(), '.openhuman');

// Config lives in a per-user subdirectory (e.g. ~/.openhuman/users/<id>/config.toml)
// when authenticated, or at the root for fresh installs. Find the right one.
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

const TRIGGER =
  'the user just finished the desktop onboarding wizard. welcome the user.';

const THREAD_ID = `judge-${randomUUID().slice(0, 8)}`;

// Scripted user messages to simulate a conversation
const USER_MESSAGES = [
  // Turn 1: trigger (auto)
  // Turn 2: user responds to the welcome
  "hey! i'm a product manager, i mostly live in slack and gmail all day. also use whatsapp for quick stuff with my team",
  // Turn 3: respond to app connection suggestion
  "yeah sure, let me connect gmail first. how does it work exactly?",
  // Turn 4: confirm connection
  "ok cool i connected gmail, what else can this thing do?",
  // Turn 5: ask about capabilities
  "that sounds awesome. can it help me with my morning routine? like summarizing what i missed overnight?",
  // Turn 6: wrapping up
  "nice, i think i'm good for now. thanks!",
];

const TURN_TIMEOUT_MS = 90_000; // 90s per turn (agent can be slow)

function dbg(...a) {
  if (DEBUG) console.log('\x1b[90m  [debug]\x1b[0m', ...a);
}
function log(msg) {
  console.log(`\x1b[36m[judge]\x1b[0m ${msg}`);
}
function err(msg) {
  console.error(`\x1b[31m[error]\x1b[0m ${msg}`);
}

// ── Reset config ───────────────────────────────────────────────────
function resetOnboarding() {
  if (!existsSync(CONFIG_PATH)) return;
  let content = readFileSync(CONFIG_PATH, 'utf-8');
  if (content.includes('chat_onboarding_completed')) {
    content = content.replace(
      /chat_onboarding_completed\s*=\s*true/,
      'chat_onboarding_completed = false'
    );
  } else {
    content = `chat_onboarding_completed = false\n${content}`;
  }
  writeFileSync(CONFIG_PATH, content, 'utf-8');
}

// ── Load socket.io-client ──────────────────────────────────────────
async function loadSocketIo() {
  const { createRequire } = await import('module');
  const dirs = [path.join(ROOT, 'app'), ROOT];
  for (const dir of dirs) {
    try {
      const require = createRequire(path.join(dir, 'package.json'));
      const mod = require('socket.io-client');
      return mod.io || mod.default || mod;
    } catch { /* fall through */ }
  }
  err('Cannot load socket.io-client');
  process.exit(1);
}

// ── Main ───────────────────────────────────────────────────────────
async function main() {
  console.log('');
  console.log('\x1b[1m  Welcome Agent — Automated Judge\x1b[0m');
  console.log('  ─────────────────────────────────────');
  console.log('');

  // Health check
  try {
    const resp = await fetch(`${CORE_URL}/health`, { signal: AbortSignal.timeout(3000) });
    if (!resp.ok) throw new Error('not ok');
  } catch {
    err(`Core not reachable at ${CORE_URL}. Start it first.`);
    process.exit(1);
  }
  log('Core server is up');

  // Reset onboarding
  resetOnboarding();
  await new Promise((r) => setTimeout(r, 500));
  log('Reset chat_onboarding_completed = false');

  const io = await loadSocketIo();
  log('Loaded socket.io-client');

  // Connect
  const socket = io(CORE_URL, {
    transports: ['websocket'],
    reconnection: false,
    timeout: 10_000,
  });

  const conversation = []; // { role, content, tools? }

  function collectTurn() {
    return new Promise((resolve, reject) => {
      let responseText = '';
      let tools = [];
      let thinkingText = '';
      const timer = setTimeout(() => {
        reject(new Error('Turn timed out'));
      }, TURN_TIMEOUT_MS);

      function onTextDelta(data) {
        if (data.delta) responseText += data.delta;
      }
      function onThinkingDelta(data) {
        if (data.delta) thinkingText += data.delta;
      }
      function onToolCall(data) {
        tools.push(data.tool_name || 'unknown');
      }
      function onChatSegment(data) {
        if (data.message) responseText += data.message;
      }
      function onDone(data) {
        clearTimeout(timer);
        cleanup();
        if (!responseText && data.full_response) {
          responseText = data.full_response;
        }
        resolve({ text: responseText.trim(), tools, thinking: thinkingText.trim() });
      }
      function onError(data) {
        clearTimeout(timer);
        cleanup();
        reject(new Error(`Chat error: ${data.message || data.error_type || 'unknown'}`));
      }
      function cleanup() {
        socket.off('text_delta', onTextDelta);
        socket.off('thinking_delta', onThinkingDelta);
        socket.off('tool_call', onToolCall);
        socket.off('chat_segment', onChatSegment);
        socket.off('chat_done', onDone);
        socket.off('chat_error', onError);
      }

      socket.on('text_delta', onTextDelta);
      socket.on('thinking_delta', onThinkingDelta);
      socket.on('tool_call', onToolCall);
      socket.on('chat_segment', onChatSegment);
      socket.on('chat_done', onDone);
      socket.on('chat_error', onError);
    });
  }

  async function sendAndCollect(message, label) {
    log(`── ${label} ──`);
    if (label !== 'Trigger') {
      console.log(`\x1b[1m  you>\x1b[0m ${message}`);
    }
    console.log('');

    const turnPromise = collectTurn();
    socket.emit('chat:start', { thread_id: THREAD_ID, message });
    const response = await turnPromise;

    console.log(`\x1b[32m  agent>\x1b[0m ${response.text}`);
    if (response.tools.length > 0) {
      console.log(`\x1b[90m  tools: ${response.tools.join(', ')}\x1b[0m`);
    }
    console.log('');

    conversation.push({ role: 'user', content: message });
    conversation.push({
      role: 'assistant',
      content: response.text,
      tools: response.tools,
    });

    return response;
  }

  // Wait for ready
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Socket ready timeout')), 10_000);
    socket.on('ready', (data) => {
      clearTimeout(timer);
      log(`Connected as ${data.sid}`);
      resolve();
    });
    socket.on('connect_error', (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });

  console.log('');
  log('Starting conversation...');
  console.log('');
  console.log('═══════════════════════════════════════════════════════');
  console.log('');

  // Turn 0: trigger
  try {
    await sendAndCollect(TRIGGER, 'Trigger');
  } catch (e) {
    err(`Trigger failed: ${e.message}`);
    socket.disconnect();
    process.exit(1);
  }

  // Subsequent turns
  for (let i = 0; i < USER_MESSAGES.length; i++) {
    // Small delay between turns to be realistic
    await new Promise((r) => setTimeout(r, 1000));
    try {
      const resp = await sendAndCollect(USER_MESSAGES[i], `Turn ${i + 2}`);
      // If agent called complete_onboarding, we're done
      if (resp.tools.includes('complete_onboarding')) {
        log('Agent called complete_onboarding — conversation ended');
        break;
      }
    } catch (e) {
      err(`Turn ${i + 2} failed: ${e.message}`);
      break;
    }
  }

  console.log('═══════════════════════════════════════════════════════');
  console.log('');

  // ── Judge ──────────────────────────────────────────────────────
  printJudgment(conversation);

  socket.disconnect();
  process.exit(0);
}

function printJudgment(conversation) {
  console.log('\x1b[1m  JUDGMENT REPORT\x1b[0m');
  console.log('  ─────────────────────────────────────');
  console.log('');

  const assistantTurns = conversation.filter((t) => t.role === 'assistant');
  const allTools = assistantTurns.flatMap((t) => t.tools || []);
  const allText = assistantTurns.map((t) => t.content).join('\n');
  const lowerText = allText.toLowerCase();

  const checks = [];

  function check(name, pass, detail) {
    checks.push({ name, pass, detail });
    const icon = pass ? '\x1b[32mPASS\x1b[0m' : '\x1b[31mFAIL\x1b[0m';
    console.log(`  [${icon}] ${name}`);
    if (detail) console.log(`\x1b[90m         ${detail}\x1b[0m`);
  }

  // 1. Did it call check_onboarding_status on first turn?
  const firstAssistant = assistantTurns[0];
  check(
    'Calls check_onboarding_status on first turn',
    firstAssistant?.tools?.includes('check_onboarding_status'),
    `Tools used: ${firstAssistant?.tools?.join(', ') || 'none'}`
  );

  // 2. Is the opener warm and invites the user to respond?
  const openerText = firstAssistant?.content || '';
  const asksQuestion = openerText.includes('?') ||
    /what would you|what do you|what's on|tell me|how can i/i.test(openerText);
  check(
    'Opener invites the user to respond',
    asksQuestion,
    `Opener: "${openerText.slice(0, 120)}..."`
  );

  // 3. Does NOT dump a checklist on turn 1
  const hasChecklist = /\d\.\s|step\s*\d|checklist|first.*second.*third/i.test(openerText);
  check(
    'Opener does NOT dump a checklist',
    !hasChecklist,
    hasChecklist ? 'Found checklist-like content in opener' : 'Clean opener'
  );

  // 4. Mentions connecting apps at some point
  const mentionsConnect =
    lowerText.includes('connect') && (lowerText.includes('app') || lowerText.includes('gmail') || lowerText.includes('slack'));
  check(
    'Mentions connecting apps during conversation',
    mentionsConnect,
    mentionsConnect ? 'Found app connection guidance' : 'Never mentioned connecting apps'
  );

  // 5. Uses <openhuman-link> for accounts/setup
  const hasAccountsLink = allText.includes('openhuman-link') && allText.includes('accounts/setup');
  check(
    'Uses <openhuman-link path="accounts/setup"> at some point',
    hasAccountsLink,
    hasAccountsLink ? 'Link found' : 'No accounts/setup link found'
  );

  // 6. Tone: no "as an AI", no "I'm OpenHuman"
  const badPhrases = ['as an ai', "i'm openhuman", 'i am openhuman', 'as an artificial'];
  const foundBad = badPhrases.find((p) => lowerText.includes(p));
  check(
    'No robotic self-identification',
    !foundBad,
    foundBad ? `Found: "${foundBad}"` : 'Clean'
  );

  // 7. No billing pitch unless user asked
  const billingPitch = lowerText.includes('billing') || lowerText.includes('subscription') || lowerText.includes('credit');
  check(
    'No unsolicited billing/subscription pitch',
    !billingPitch,
    billingPitch ? 'Found billing/subscription mention' : 'Clean'
  );

  // 8. No em-dashes
  const hasEmDash = allText.includes('\u2014');
  check(
    'No em-dashes in responses',
    !hasEmDash,
    hasEmDash ? 'Found em-dash characters' : 'Clean'
  );

  // 9. Responds to user interests (slack/gmail mentioned by user)
  const respondsToInterests =
    lowerText.includes('slack') || lowerText.includes('gmail') || lowerText.includes('whatsapp');
  check(
    'References apps the user mentioned (slack/gmail/whatsapp)',
    respondsToInterests,
    respondsToInterests ? 'Agent engaged with user interests' : 'Did not reference user apps'
  );

  // 10. Mentions capabilities organically (morning briefing etc)
  const mentionsCapabilities =
    lowerText.includes('morning') ||
    lowerText.includes('briefing') ||
    lowerText.includes('action item') ||
    lowerText.includes('summary') ||
    lowerText.includes('monitor');
  check(
    'Educates about capabilities when relevant',
    mentionsCapabilities,
    mentionsCapabilities ? 'Found capability education' : 'No capabilities mentioned'
  );

  // 11. Discord mentioned casually (not as mandatory step)
  const mentionsDiscord = lowerText.includes('discord');
  const discordForced = lowerText.includes('must join') || lowerText.includes('need to join discord');
  check(
    'Discord mentioned casually (not forced)',
    !discordForced,
    mentionsDiscord
      ? discordForced ? 'Discord was forced' : 'Discord mentioned casually'
      : 'Discord not mentioned (acceptable if conversation ended early)'
  );

  // 12. No JSON/code fences in responses
  const hasCodeFence = allText.includes('```') || allText.includes('{"');
  check(
    'No JSON or code fences in responses',
    !hasCodeFence,
    hasCodeFence ? 'Found code/JSON in output' : 'Clean prose'
  );

  // 13. Messages are short (avg < 300 chars per turn)
  const avgLen = assistantTurns.length > 0
    ? assistantTurns.reduce((sum, t) => sum + t.content.length, 0) / assistantTurns.length
    : 0;
  check(
    'Messages are concise (avg < 300 chars)',
    avgLen < 300,
    `Average message length: ${Math.round(avgLen)} chars`
  );

  // Summary
  const passed = checks.filter((c) => c.pass).length;
  const total = checks.length;
  console.log('');
  console.log(`  ─────────────────────────────────────`);
  console.log(`  \x1b[1mScore: ${passed}/${total}\x1b[0m`);
  if (passed === total) {
    console.log('  \x1b[32mAll checks passed!\x1b[0m');
  } else {
    console.log(`  \x1b[33m${total - passed} check(s) failed — review above\x1b[0m`);
  }
  console.log('');
}

main().catch((e) => {
  err(e.stack || e.message);
  process.exit(1);
});
