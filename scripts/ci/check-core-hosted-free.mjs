#!/usr/bin/env node
// Keeps the core free of anything TinyHumans-hosted.
//
// `crates/openhuman-core` has no backend module, no hosted port and no hosted
// route: the TinyHumans layer (`crates/openhuman-tinyhumans`) owns the
// transport, the authenticated client, the routes and the hosted providers,
// and plugs into the core only through generic registries. This check scans
// the core's Rust source (comments and `_tests.rs` files excluded) and fails
// on
//   - `crate::backend` (the deleted module),
//   - `tinyhumans.ai` (a hosted hostname), and
//   - hosted route literals: `agent-integrations`, `/voice-agent`,
//     `/openai/v1`, `/telemetry/langfuse`, `/memory/free-period`, `/teams/me`.
//
// Usage: check-core-hosted-free.mjs [repo-root]
// Exit 0 when clean, 1 on a violation, 2 when the source tree is unreadable.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const CORE_SRC = 'crates/openhuman-core/src';

export const FORBIDDEN = [
  { name: 'crate::backend', regex: /\bcrate::backend\b/ },
  { name: 'tinyhumans.ai', regex: /tinyhumans\.ai/ },
  { name: 'agent-integrations', regex: /agent-integrations/ },
  { name: '/voice-agent', regex: /\/voice-agent/ },
  { name: '/openai/v1', regex: /\/openai\/v1/ },
  { name: '/telemetry/langfuse', regex: /\/telemetry\/langfuse/ },
  { name: '/memory/free-period', regex: /\/memory\/free-period/ },
  { name: '/teams/me', regex: /\/teams\/me/ },
];

/**
 * Blank out Rust comments (`//`, nested block comments, doc comments) while
 * keeping string and char literals and every newline, so reported line numbers
 * stay true. Hosted literals live in strings, which must still be scanned.
 */
export function stripRustComments(src) {
  let out = '';
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    const next = src[i + 1];
    if (c === '/' && next === '/') {
      while (i < n && src[i] !== '\n') i++;
      continue;
    }
    if (c === '/' && next === '*') {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (src[i] === '/' && src[i + 1] === '*') {
          depth++;
          i += 2;
        } else if (src[i] === '*' && src[i + 1] === '/') {
          depth--;
          i += 2;
        } else {
          if (src[i] === '\n') out += '\n';
          i++;
        }
      }
      continue;
    }
    // Raw string: r"..." / r#"..."# / br#"..."#.
    const raw = /^b?r(#*)"/.exec(src.slice(i, i + 40));
    if (raw && (i === 0 || !/[A-Za-z0-9_]/.test(src[i - 1]))) {
      const close = '"' + raw[1];
      const end = src.indexOf(close, i + raw[0].length);
      const stop = end === -1 ? n : end + close.length;
      out += src.slice(i, stop);
      i = stop;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === '\\' ? 2 : 1;
      out += src.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (c === "'") {
      // A char literal ('"', '\'', 'x') versus a lifetime ('a).
      const lit = /^'(?:\\.[^']*|[^\\'])'/.exec(src.slice(i, i + 12));
      if (lit) {
        out += lit[0];
        i += lit[0].length;
        continue;
      }
    }
    out += c;
    i++;
  }
  return out;
}

/** Violations `[{ file, line, name, text }]` in one file's source. */
export function scanSource(file, src) {
  const hits = [];
  const lines = stripRustComments(src).split('\n');
  lines.forEach((text, idx) => {
    for (const { name, regex } of FORBIDDEN) {
      if (regex.test(text)) hits.push({ file, line: idx + 1, name, text: text.trim() });
    }
  });
  return hits;
}

function* rustFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* rustFiles(path);
    else if (entry.name.endsWith('.rs') && !entry.name.endsWith('_tests.rs')) yield path;
  }
}

/** Scan the whole core source tree under `root`. */
export function checkRepository(root) {
  const src = join(root, CORE_SRC);
  if (!existsSync(src)) throw new Error(`${CORE_SRC} not found under ${root}`);
  const hits = [];
  for (const file of rustFiles(src)) {
    hits.push(...scanSource(relative(root, file), readFileSync(file, 'utf8')));
  }
  return hits;
}

export function formatReport(hits) {
  if (hits.length === 0) return 'core-hosted-free: ok (no hosted references in the core)';
  return [
    'core-hosted-free: FAILED. The core must not name the hosted backend.',
    ...hits.map(h => `  - ${h.file}:${h.line}: ${h.name}: ${h.text}`),
    'Move hosted code into crates/openhuman-tinyhumans and plug it in through a generic core registry.',
  ].join('\n');
}

function main() {
  const root = resolve(process.argv[2] ?? resolve(dirname(fileURLToPath(import.meta.url)), '../..'));
  let hits;
  try {
    hits = checkRepository(root);
  } catch (error) {
    console.error(`core-hosted-free: cannot read the core source: ${error.message}`);
    process.exit(2);
  }
  console.log(formatReport(hits));
  process.exit(hits.length === 0 ? 0 : 1);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) main();
