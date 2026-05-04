#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────────────
// test-onboarding-stress.mjs
//
// Runs 25 diverse onboarding scenarios, judges each one, and writes
// a full report to docs/ONBOARDING-TEST-RESULTS.md
// ──────────────────────────────────────────────────────────────────────

import { existsSync, readFileSync, writeFileSync, readdirSync } from 'fs';
import { homedir } from 'os';
import path from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');

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
    } catch {}
  }
  return path.join(OPENHUMAN_HOME, 'config.toml');
}

const CONFIG_PATH = findConfigPath();
const TRIGGER = 'the user just finished the desktop onboarding wizard. welcome the user.';
const TURN_TIMEOUT_MS = 120_000;

// ── 25 diverse test scenarios ──────────────────────────────────────
const SCENARIOS = [
  {
    name: 'PM in Slack/Gmail',
    messages: [
      "hey! i'm a product manager, mostly in slack and gmail all day",
      "yeah sure, how do i connect slack?",
      "done, connected it. what else can this do?",
      "cool, i think that's enough for now. thanks!",
    ],
  },
  {
    name: 'Developer, minimal talker',
    messages: [
      "i'm a dev. mostly github and vscode",
      "ok",
      "sure connected github",
      "done",
    ],
  },
  {
    name: 'Curious user, lots of questions',
    messages: [
      "what exactly is this app? like what can it actually do for me?",
      "interesting. can it read my emails?",
      "how is that different from just using gmail?",
      "ok makes sense. i connected gmail. what about privacy, is my data safe?",
      "alright cool, i'm good",
    ],
  },
  {
    name: 'Impatient user wants to skip',
    messages: [
      "can we skip all this setup stuff? i just want to use the app",
      "fine, what do i need to do?",
      "ok connected whatsapp. now can i use it?",
    ],
  },
  {
    name: 'Non-technical user',
    messages: [
      "hi! i'm not very techy but my friend told me to try this. i mostly use whatsapp and email",
      "what does connecting my apps mean? is it safe?",
      "ok i trust you. i connected my gmail",
      "what can you help me with now?",
      "that sounds great, thanks!",
    ],
  },
  {
    name: 'Enterprise user, security focused',
    messages: [
      "i'm an IT manager at a mid-size company. we're evaluating this for our team",
      "what data do you store? where does it go?",
      "ok connected gmail via oauth. what integrations do you support?",
      "can multiple team members use this?",
      "alright, i've seen enough for now",
    ],
  },
  {
    name: 'Student on a budget',
    messages: [
      "hey i'm a college student, just trying to stay organized with classes and stuff",
      "i use google calendar and gmail mostly. sometimes discord for study groups",
      "connected gmail! does this cost money?",
      "ok cool, thanks!",
    ],
  },
  {
    name: 'Freelancer juggling clients',
    messages: [
      "i'm a freelance designer. i have like 5 different clients all using different tools. slack for some, email for others, whatsapp for a couple",
      "that sounds perfect. let me connect slack first",
      "done. can it help me track which client said what across all these channels?",
      "amazing. i'm sold. thanks!",
    ],
  },
  {
    name: 'User who just says hi',
    messages: [
      "hi",
      "what should i do?",
      "ok connected gmail",
      "cool thanks bye",
    ],
  },
  {
    name: 'User reports a bug',
    messages: [
      "hey the connect button isn't working for me",
      "i clicked on connect apps but nothing happens",
      "ok i'll try again later. i managed to connect gmail through the settings though",
      "alright we're good",
    ],
  },
  {
    name: 'WhatsApp-heavy user',
    messages: [
      "i basically live on whatsapp. all my work chats, family groups, everything",
      "yeah let me connect it. done!",
      "can you read my whatsapp messages and summarize them?",
      "perfect, that's all i need. thanks!",
    ],
  },
  {
    name: 'User wants morning briefing immediately',
    messages: [
      "i heard this can do morning briefings? that's why i downloaded it",
      "yeah i connected gmail already during the setup wizard",
      "how do i set up the morning briefing?",
      "awesome, can't wait to try it tomorrow. that's all for now!",
    ],
  },
  {
    name: 'Skeptical user',
    messages: [
      "another ai app huh. what makes this different from chatgpt?",
      "ok but how do i know you won't spam my contacts or something?",
      "fine i'll try connecting gmail. done",
      "we'll see. bye",
    ],
  },
  {
    name: 'Power user wants everything',
    messages: [
      "i want to connect everything. gmail, slack, whatsapp, telegram, discord, notion, github, calendar",
      "done, connected gmail and slack so far. how do i add the rest?",
      "connected telegram and whatsapp too. can you now monitor all of them at once?",
      "set up morning briefings and auto-triage my inbox please",
      "this is sick. thanks!",
    ],
  },
  {
    name: 'User in a hurry',
    messages: [
      "i have 2 minutes. what do i absolutely need to do?",
      "done, connected gmail",
      "great, gotta go. bye!",
    ],
  },
  {
    name: 'Telegram-first user',
    messages: [
      "i want to use this mainly through telegram. is that possible?",
      "cool. i connected telegram already",
      "can you reach me there when something important happens?",
      "perfect. that's it for now",
    ],
  },
  {
    name: 'User asks about pricing',
    messages: [
      "how much does this cost?",
      "ok. what do i get for free?",
      "alright let me connect gmail and try it out",
      "connected. thanks!",
    ],
  },
  {
    name: 'User speaks broken English',
    messages: [
      "hello i am new here. my english not so good. i use whatsapp and email for work",
      "ok i connect the gmail now. done",
      "what can you do help me?",
      "ok thank you very much!",
    ],
  },
  {
    name: 'User wants automation',
    messages: [
      "i want to automate as much of my workflow as possible. i'm drowning in notifications",
      "i use slack, gmail, and notion. can you auto-sort my notifications?",
      "connected all three. what automation can you set up?",
      "set up whatever you think makes sense. i trust you",
      "thanks, this is exactly what i needed",
    ],
  },
  {
    name: 'User only wants one thing',
    messages: [
      "i only care about email. can you help me manage my inbox?",
      "connected gmail. now what?",
      "can you triage my unread right now?",
      "ok sure, let me finish setup first then. we're done?",
    ],
  },
  {
    name: 'User compares to competitors',
    messages: [
      "i've tried notion ai and copilot. how is this different?",
      "interesting. the cross-app thing is unique. let me try connecting slack",
      "done. can you show me something slack-specific?",
      "ok cool, i get it now. thanks",
    ],
  },
  {
    name: 'Verbose storyteller',
    messages: [
      "so basically my problem is that i have way too many apps open at once. like right now i have gmail, slack, whatsapp web, telegram, notion, and google calendar all in different tabs and i'm constantly switching between them and losing track of conversations. my boss messages me on slack but then my client follows up on whatsapp about the same project and i forget to connect the dots",
      "yeah that's exactly it. let me connect slack and gmail",
      "done! this is promising",
      "i'm good for now, thanks for listening to my rant haha",
    ],
  },
  {
    name: 'User wants voice features',
    messages: [
      "can i talk to this thing with my voice?",
      "cool. for now let me just connect gmail",
      "done. when will voice be fully ready?",
      "alright, thanks!",
    ],
  },
  {
    name: 'Team lead evaluating for team',
    messages: [
      "i'm a team lead looking at this for my team of 12. we all use slack and google workspace",
      "how does the team feature work?",
      "interesting. i connected gmail for now to test it myself first",
      "can each team member have their own setup?",
      "makes sense. i'll report back to the team. thanks!",
    ],
  },
  {
    name: 'User who immediately connects app',
    messages: [
      "i already connected whatsapp and gmail during the wizard",
      "what can you do for me now?",
      "cool can you check my unread emails?",
      "ok let's finish the setup first. we done?",
    ],
  },
];

// ── Helpers ────────────────────────────────────────────────────────
function log(msg) {
  process.stdout.write(`\x1b[36m[stress]\x1b[0m ${msg}\n`);
}

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

async function loadSocketIo() {
  const { createRequire } = await import('module');
  for (const dir of [path.join(ROOT, 'app'), ROOT]) {
    try {
      const require = createRequire(path.join(dir, 'package.json'));
      const mod = require('socket.io-client');
      return mod.io || mod.default || mod;
    } catch {}
  }
  process.exit(1);
}

// ── Run one scenario ───────────────────────────────────────────────
async function runScenario(io, scenario, index) {
  resetOnboarding();
  await new Promise((r) => setTimeout(r, 800));

  const threadId = `stress-${index}-${randomUUID().slice(0, 6)}`;
  const conversation = [];

  const socket = io(CORE_URL, {
    transports: ['websocket'],
    reconnection: false,
    timeout: 10_000,
  });

  // Wait for ready
  const clientId = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('ready timeout')), 10_000);
    socket.on('ready', (d) => { clearTimeout(timer); resolve(d.sid); });
    socket.on('connect_error', (e) => { clearTimeout(timer); reject(e); });
  });

  function collectTurn() {
    return new Promise((resolve, reject) => {
      let text = '';
      let tools = [];
      const timer = setTimeout(() => {
        cleanup();
        resolve({ text: text.trim() || '(timeout - no response)', tools });
      }, TURN_TIMEOUT_MS);

      function onDelta(d) { if (d.delta) text += d.delta; }
      function onSeg(d) { if (d.message) text += d.message; }
      function onTool(d) { tools.push(d.tool_name || 'unknown'); }
      function onDone(d) {
        clearTimeout(timer); cleanup();
        if (!text && d.full_response) text = d.full_response;
        resolve({ text: text.trim(), tools });
      }
      function onErr(d) {
        clearTimeout(timer); cleanup();
        resolve({ text: `(error: ${d.message || d.error_type || 'unknown'})`, tools });
      }
      function cleanup() {
        socket.off('text_delta', onDelta);
        socket.off('chat_segment', onSeg);
        socket.off('tool_call', onTool);
        socket.off('chat_done', onDone);
        socket.off('chat_error', onErr);
      }

      socket.on('text_delta', onDelta);
      socket.on('chat_segment', onSeg);
      socket.on('tool_call', onTool);
      socket.on('chat_done', onDone);
      socket.on('chat_error', onErr);
    });
  }

  async function send(message) {
    const p = collectTurn();
    socket.emit('chat:start', { thread_id: threadId, message });
    return await p;
  }

  // Trigger
  const triggerResp = await send(TRIGGER);
  conversation.push({ role: 'trigger', content: TRIGGER });
  conversation.push({ role: 'assistant', content: triggerResp.text, tools: triggerResp.tools });

  // User turns
  for (const msg of scenario.messages) {
    await new Promise((r) => setTimeout(r, 500));
    const resp = await send(msg);
    conversation.push({ role: 'user', content: msg });
    conversation.push({ role: 'assistant', content: resp.text, tools: resp.tools });
    if (resp.tools.includes('complete_onboarding')) break;
  }

  socket.disconnect();
  return { conversation, clientId };
}

// ── Judge a conversation ───────────────────────────────────────────
function judge(conversation) {
  const assistantTurns = conversation.filter((t) => t.role === 'assistant');
  const allTools = assistantTurns.flatMap((t) => t.tools || []);
  const allText = assistantTurns.map((t) => t.content).join('\n');
  const lower = allText.toLowerCase();
  const firstA = assistantTurns[0];
  const opener = firstA?.content || '';

  // Context flags for conditional checks
  const userText = conversation
    .filter((t) => t.role === 'user')
    .map((t) => t.content)
    .join('\n')
    .toLowerCase();
  const agentGuidedConnection = assistantTurns.some((t) =>
    /\bconnect\b/.test(t.content) || t.content.includes('accounts/setup')
  );
  const userAskedPricing = /\b(price|pricing|cost|billing|subscription|how much)\b/i.test(userText);

  const checks = {};

  checks['check_onboarding_status_first_turn'] = firstA?.tools?.includes('check_onboarding_status') || false;
  checks['opener_invites_response'] = opener.includes('?') || /what would you|what do you|what.?s on|tell me|how can i/i.test(opener);
  checks['no_checklist_dump'] = !/\d\.\s|step\s*\d|checklist|first.*second.*third/i.test(opener);
  checks['mentions_connecting_apps'] = lower.includes('connect') && (lower.includes('app') || lower.includes('gmail') || lower.includes('slack') || lower.includes('whatsapp'));
  // Only check pill usage when the agent actually guided a connection
  checks['uses_openhuman_link'] = !agentGuidedConnection || (allText.includes('openhuman-link') && allText.includes('accounts/setup'));
  checks['no_robotic_self_id'] = !['as an ai', "i'm openhuman", 'i am openhuman'].some((p) => lower.includes(p));
  // Allow billing mentions when the user explicitly asked about pricing
  checks['no_billing_pitch'] = userAskedPricing || !(lower.includes('billing') || lower.includes('subscription') || (lower.includes('credit') && lower.includes('trial')));
  checks['no_em_dashes'] = !allText.includes('\u2014');
  checks['references_user_apps'] = lower.includes('slack') || lower.includes('gmail') || lower.includes('whatsapp') || lower.includes('telegram') || lower.includes('notion') || lower.includes('github') || lower.includes('discord');
  checks['educates_capabilities'] = lower.includes('morning') || lower.includes('briefing') || lower.includes('action item') || lower.includes('summary') || lower.includes('summariz') || lower.includes('monitor') || lower.includes('triage') || lower.includes('automat');
  checks['discord_not_forced'] = !(lower.includes('must join') || lower.includes('need to join discord'));
  checks['no_json_or_code'] = !(allText.includes('```') || allText.includes('{"'));
  checks['messages_concise'] = assistantTurns.length > 0 ? (assistantTurns.reduce((s, t) => s + t.content.length, 0) / assistantTurns.length < 350) : true;
  checks['calls_complete_onboarding'] = allTools.includes('complete_onboarding');
  checks['no_markdown_formatting'] = !(allText.includes('**') || allText.includes('## ') || /^- /m.test(allText) || /^\d+\. /m.test(allText));

  const passed = Object.values(checks).filter(Boolean).length;
  const total = Object.keys(checks).length;

  return { checks, passed, total };
}

// ── Main ───────────────────────────────────────────────────────────
async function main() {
  console.log('');
  console.log('\x1b[1m  Welcome Agent Stress Test — 25 Scenarios\x1b[0m');
  console.log('  ════════════════════════════════════════════');
  console.log('');

  // Health check
  try {
    const r = await fetch(`${CORE_URL}/health`, { signal: AbortSignal.timeout(3000) });
    if (!r.ok) throw new Error();
  } catch {
    console.error('Core not reachable. Start it first.');
    process.exit(1);
  }
  log(`Core up at ${CORE_URL}`);
  log(`Config at ${CONFIG_PATH}`);
  console.log('');

  const io = await loadSocketIo();
  const results = [];

  for (let i = 0; i < SCENARIOS.length; i++) {
    const s = SCENARIOS[i];
    const label = `[${i + 1}/${SCENARIOS.length}] ${s.name}`;
    process.stdout.write(`\x1b[36m[stress]\x1b[0m ${label}...`);

    try {
      const { conversation } = await runScenario(io, s, i);
      const judgment = judge(conversation);
      results.push({ scenario: s, conversation, judgment });

      const color = judgment.passed === judgment.total ? '\x1b[32m' : judgment.passed >= judgment.total - 2 ? '\x1b[33m' : '\x1b[31m';
      process.stdout.write(` ${color}${judgment.passed}/${judgment.total}\x1b[0m\n`);
    } catch (e) {
      process.stdout.write(` \x1b[31mERROR: ${e.message}\x1b[0m\n`);
      results.push({ scenario: s, conversation: [], judgment: { checks: {}, passed: 0, total: 15, error: e.message } });
    }

    // Cool down between scenarios
    await new Promise((r) => setTimeout(r, 2000));
  }

  // ── Generate report ────────────────────────────────────────────
  const report = generateReport(results);
  const outPath = path.join(ROOT, 'docs', 'ONBOARDING-TEST-RESULTS.md');
  writeFileSync(outPath, report, 'utf-8');
  log(`Report written to ${outPath}`);

  // Summary
  console.log('');
  console.log('\x1b[1m  SUMMARY\x1b[0m');
  console.log('  ─────────────────────────────');
  const totalPassed = results.reduce((s, r) => s + r.judgment.passed, 0);
  const totalChecks = results.reduce((s, r) => s + r.judgment.total, 0);
  const perfect = results.filter((r) => r.judgment.passed === r.judgment.total).length;
  const avgScore = (totalPassed / totalChecks * 100).toFixed(1);
  console.log(`  Scenarios: ${results.length}`);
  console.log(`  Perfect scores: ${perfect}/${results.length}`);
  console.log(`  Overall: ${totalPassed}/${totalChecks} checks passed (${avgScore}%)`);
  console.log('');

  // Per-check stats
  const checkStats = {};
  for (const r of results) {
    for (const [k, v] of Object.entries(r.judgment.checks)) {
      if (!checkStats[k]) checkStats[k] = { pass: 0, fail: 0 };
      if (v) checkStats[k].pass++; else checkStats[k].fail++;
    }
  }
  console.log('  \x1b[1mPer-check pass rate:\x1b[0m');
  for (const [k, v] of Object.entries(checkStats).sort((a, b) => a[1].pass - b[1].pass)) {
    const pct = ((v.pass / (v.pass + v.fail)) * 100).toFixed(0);
    const color = pct === '100' ? '\x1b[32m' : parseInt(pct) >= 80 ? '\x1b[33m' : '\x1b[31m';
    console.log(`  ${color}${pct.padStart(4)}%\x1b[0m  ${k} (${v.pass}/${v.pass + v.fail})`);
  }
  console.log('');
}

function generateReport(results) {
  const lines = [];
  const now = new Date().toISOString().slice(0, 19).replace('T', ' ');

  lines.push('# Onboarding Agent Test Results');
  lines.push('');
  lines.push(`Generated: ${now}`);
  lines.push('');

  // Summary table
  const totalPassed = results.reduce((s, r) => s + r.judgment.passed, 0);
  const totalChecks = results.reduce((s, r) => s + r.judgment.total, 0);
  const perfect = results.filter((r) => r.judgment.passed === r.judgment.total).length;

  lines.push('## Summary');
  lines.push('');
  lines.push(`- **Scenarios run:** ${results.length}`);
  lines.push(`- **Perfect scores:** ${perfect}/${results.length}`);
  lines.push(`- **Overall pass rate:** ${totalPassed}/${totalChecks} (${(totalPassed / totalChecks * 100).toFixed(1)}%)`);
  lines.push('');

  // Scorecard table
  lines.push('## Scorecard');
  lines.push('');
  lines.push('| # | Scenario | Score | Status |');
  lines.push('|---|----------|-------|--------|');
  for (let i = 0; i < results.length; i++) {
    const r = results[i];
    const status = r.judgment.error ? 'ERROR' : r.judgment.passed === r.judgment.total ? 'PASS' : r.judgment.passed >= r.judgment.total - 2 ? 'WARN' : 'FAIL';
    lines.push(`| ${i + 1} | ${r.scenario.name} | ${r.judgment.passed}/${r.judgment.total} | ${status} |`);
  }
  lines.push('');

  // Per-check pass rate
  const checkStats = {};
  for (const r of results) {
    for (const [k, v] of Object.entries(r.judgment.checks)) {
      if (!checkStats[k]) checkStats[k] = { pass: 0, fail: 0 };
      if (v) checkStats[k].pass++; else checkStats[k].fail++;
    }
  }
  lines.push('## Per-Check Pass Rate');
  lines.push('');
  lines.push('| Check | Pass Rate | Passed | Failed |');
  lines.push('|-------|-----------|--------|--------|');
  for (const [k, v] of Object.entries(checkStats).sort((a, b) => (a[1].pass / (a[1].pass + a[1].fail)) - (b[1].pass / (b[1].pass + b[1].fail)))) {
    const pct = ((v.pass / (v.pass + v.fail)) * 100).toFixed(0);
    lines.push(`| ${k} | ${pct}% | ${v.pass} | ${v.fail} |`);
  }
  lines.push('');

  // Redact sensitive URLs from report output
  function sanitize(text) {
    return text
      .replace(/https?:\/\/backend\.composio\.dev\/api\/[^\s)]+/g, '[REDACTED_COMPOSIO_URL]')
      .replace(/https?:\/\/[^\s)]*composio[^\s)]*\/connect[^\s)]*/gi, '[REDACTED_COMPOSIO_URL]')
      .replace(/Bearer\s+[A-Za-z0-9._-]+/g, 'Bearer [REDACTED]');
  }

  // Full conversations
  lines.push('## Full Conversations');
  lines.push('');
  for (let i = 0; i < results.length; i++) {
    const r = results[i];
    lines.push(`### ${i + 1}. ${r.scenario.name} (${r.judgment.passed}/${r.judgment.total})`);
    lines.push('');

    if (r.judgment.error) {
      lines.push(`**ERROR:** ${r.judgment.error}`);
      lines.push('');
      continue;
    }

    // Failed checks
    const failed = Object.entries(r.judgment.checks).filter(([, v]) => !v).map(([k]) => k);
    if (failed.length > 0) {
      lines.push(`**Failed checks:** ${failed.join(', ')}`);
      lines.push('');
    }

    // Conversation
    lines.push('```');
    for (const turn of r.conversation) {
      if (turn.role === 'trigger') {
        lines.push(`[trigger] ${sanitize(turn.content)}`);
      } else if (turn.role === 'user') {
        lines.push('');
        lines.push(`you> ${sanitize(turn.content)}`);
      } else {
        const tools = turn.tools?.length ? ` [tools: ${turn.tools.join(', ')}]` : '';
        lines.push(`agent> ${sanitize(turn.content)}${tools}`);
      }
    }
    lines.push('```');
    lines.push('');
  }

  return lines.join('\n');
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
