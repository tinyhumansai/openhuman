#!/usr/bin/env node
/**
 * Audit KV-cache prefix stability across a captured sequence of inference
 * requests.
 *
 * Record a sequence first:
 *
 *   CAPTURE_ALL=1 node scripts/debug/capture-first-inference.mjs
 *
 * then point the core's BACKEND_URL at the proxy, run a multi-turn session, and:
 *
 *   node scripts/debug/audit-inference-prefix.mjs target/debug-logs/inference-sequence
 *
 * # What is actually being measured
 *
 * Every prefix cache in production — OpenAI's automatic one, Anthropic's
 * explicit `cache_control`, DeepSeek's context cache, and every vLLM/SGLang
 * radix cache behind a self-hosted model — keys on a *rendered prefix* in which
 * the tool definitions come BEFORE the conversation. They have to: a chat
 * template has to emit the tool catalogue somewhere the model reads it before
 * the first user turn.
 *
 * So the JSON key order of the request body (`messages` before `tools`, because
 * that is the order the struct declares its fields in) says nothing about cache
 * behaviour, and neither does where a renderer chooses to print the tool
 * schemas. What decides it is:
 *
 *     cacheable prefix  =  canonical(tools) ++ messages[0] ++ messages[1] ++ ...
 *
 * up to the first byte that differs from the previous request. One changed tool
 * schema therefore invalidates the *entire* prefix, system prompt included —
 * which is why freezing the system prompt while letting the tool array move is
 * a contradiction rather than an optimisation.
 *
 * This script reports the first divergence and attributes it, so a regression
 * names the tool or the message that moved instead of a token count that got
 * worse.
 */

import fs from 'node:fs';
import path from 'node:path';

const dir = path.resolve(process.argv[2] || 'target/debug-logs/inference-sequence');

if (!fs.existsSync(dir)) {
  process.stderr.write(`[prefix-audit] no capture directory at ${dir}\n`);
  process.exit(2);
}

// `req-<N>.json` pads N to a minimum of 3 digits (see capture-first-inference.mjs),
// so a lexicographic sort places `req-1000.json` before `req-999.json` once a
// sequence runs past 999 captures. Sort by the numeric index so adjacent
// comparisons always compare consecutive requests.
function sequenceIndex(filename) {
  const match = /^req-(\d+)\.json$/.exec(filename);
  return match ? Number.parseInt(match[1], 10) : Number.POSITIVE_INFINITY;
}

const files = fs
  .readdirSync(dir)
  .filter(f => f.endsWith('.json'))
  .sort((a, b) => sequenceIndex(a) - sequenceIndex(b));

if (files.length < 2) {
  process.stderr.write(
    `[prefix-audit] need at least 2 captured requests in ${dir}, found ${files.length}\n`
  );
  process.exit(2);
}

const requests = files.map(f => ({
  name: f,
  body: JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')),
}));

/** Canonical bytes for one tool schema. Key order is the capture's own. */
const toolBytes = tool => JSON.stringify(tool);
/** Canonical bytes for one message. */
const messageBytes = message => JSON.stringify(message);

const toolName = tool => tool?.function?.name ?? tool?.name ?? '<unnamed>';

/**
 * The cacheable prefix as an ordered list of labelled chunks: tools first, then
 * messages. Comparing two of these chunk-wise finds the first divergence and
 * says what it was.
 */
function prefixChunks(body) {
  const chunks = [];
  for (const tool of body.tools ?? []) {
    chunks.push({ kind: 'tool', label: toolName(tool), bytes: toolBytes(tool) });
  }
  (body.messages ?? []).forEach((message, index) => {
    chunks.push({
      kind: 'message',
      label: `${index}:${message.role}`,
      bytes: messageBytes(message),
    });
  });
  return chunks;
}

function totalBytes(chunks) {
  return chunks.reduce((sum, chunk) => sum + chunk.bytes.length, 0);
}

/** Bytes shared, counting only whole chunks, up to the first difference. */
function sharedPrefix(a, b) {
  let index = 0;
  let bytes = 0;
  while (index < a.length && index < b.length && a[index].bytes === b[index].bytes) {
    bytes += a[index].bytes.length;
    index += 1;
  }
  return { index, bytes };
}

/** Attribute a tool-block change: reorder, edit, addition or removal. */
function describeToolDrift(previous, next) {
  const prevTools = previous.body.tools ?? [];
  const nextTools = next.body.tools ?? [];
  const prevNames = prevTools.map(toolName);
  const nextNames = nextTools.map(toolName);

  const added = nextNames.filter(n => !prevNames.includes(n));
  const removed = prevNames.filter(n => !nextNames.includes(n));

  const notes = [];
  if (added.length) notes.push(`added: ${added.join(', ')}`);
  if (removed.length) notes.push(`removed: ${removed.join(', ')}`);

  if (!added.length && !removed.length) {
    const reordered = prevNames.join(' | ') !== nextNames.join(' | ');
    if (reordered) {
      notes.push('same tool set, DIFFERENT ORDER (pure cache loss, no capability change)');
    }
    const edited = [];
    for (const name of prevNames) {
      const before = prevTools.find(t => toolName(t) === name);
      const after = nextTools.find(t => toolName(t) === name);
      if (after && toolBytes(before) !== toolBytes(after)) edited.push(name);
    }
    if (edited.length) notes.push(`schema text changed: ${edited.join(', ')}`);
  }
  return notes;
}

let failures = 0;
console.log(`[prefix-audit] ${requests.length} captured requests from ${dir}\n`);

for (let i = 1; i < requests.length; i += 1) {
  const previous = requests[i - 1];
  const next = requests[i];
  const a = prefixChunks(previous.body);
  const b = prefixChunks(next.body);
  const shared = sharedPrefix(a, b);
  const reusable = totalBytes(a);
  const pct = reusable === 0 ? 0 : (shared.bytes / reusable) * 100;

  const header = `${previous.name} -> ${next.name}`;
  const toolsIdentical =
    JSON.stringify(previous.body.tools ?? []) === JSON.stringify(next.body.tools ?? []);

  if (shared.index === a.length) {
    console.log(
      `  OK    ${header}: full previous prefix reused ` +
        `(${shared.bytes} B, ${b.length - a.length} new chunk(s) appended)`
    );
    continue;
  }

  failures += 1;
  const firstDiff = a[shared.index];
  const otherDiff = b[shared.index];
  console.log(`  BREAK ${header}: prefix diverges at chunk ${shared.index}`);
  console.log(
    `        reused ${shared.bytes}/${reusable} B (${pct.toFixed(1)}%), ` +
      `lost ${reusable - shared.bytes} B of cacheable prefix`
  );
  console.log(
    `        first differing chunk: ${firstDiff?.kind} "${firstDiff?.label}" ` +
      `vs ${otherDiff?.kind} "${otherDiff?.label}"`
  );
  if (!toolsIdentical) {
    for (const note of describeToolDrift(previous, next)) {
      console.log(`        tools: ${note}`);
    }
    console.log(
      "        NOTE: the tool block precedes the conversation in every provider's " +
        'rendered prefix, so this invalidates the system prompt too.'
    );
  }
  console.log('');
}

console.log(
  `\n[prefix-audit] ${requests.length - 1} transition(s), ${failures} prefix break(s)`
);
process.exit(failures === 0 ? 0 : 1);
