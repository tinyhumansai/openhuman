import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  CORE_SRC,
  checkRepository,
  formatReport,
  scanSource,
  stripRustComments,
} from '../ci/check-core-hosted-free.mjs';

const CHECKER = resolve(dirname(fileURLToPath(import.meta.url)), '../ci/check-core-hosted-free.mjs');

function fixture(files) {
  const root = mkdtempSync(join(tmpdir(), 'core-hosted-free-'));
  for (const [path, body] of Object.entries(files)) {
    const full = join(root, CORE_SRC, path);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, body);
  }
  return root;
}

test('comments are ignored but string literals are scanned', () => {
  const src = [
    '// crate::backend in a line comment',
    '/// "https://api.tinyhumans.ai" in a doc comment',
    '/* /teams/me /* nested */ still a comment */',
    'let ok = 1;',
    'let url = "https://api.tinyhumans.ai/x";',
    'let raw = r#"/voice-agent/start"#;',
    "let quote = '\"'; let route = \"/openai/v1/chat\";",
  ].join('\n');
  const hits = scanSource('x.rs', src);
  assert.deepEqual(
    hits.map(h => [h.line, h.name]),
    [
      [5, 'tinyhumans.ai'],
      [6, '/voice-agent'],
      [7, '/openai/v1'],
    ]
  );
});

test('every forbidden pattern is detected in code', () => {
  const code = [
    'use crate::backend::BackendClient;',
    'const A: &str = "agent-integrations/x";',
    'const B: &str = "/telemetry/langfuse/ingestion";',
    'const C: &str = "/memory/free-period";',
    'const D: &str = "/teams/me/usage";',
  ].join('\n');
  assert.equal(scanSource('x.rs', code).length, 5);
});

test('stripping keeps line numbers', () => {
  const src = 'a\n/* one\ntwo */\nb\n';
  assert.equal(stripRustComments(src).split('\n').length, src.split('\n').length);
});

test('_tests.rs files are skipped and a clean tree passes', () => {
  const root = fixture({
    'lib.rs': 'pub fn f() {}\n',
    'thing/mod.rs': '// crate::backend was removed\n',
    'thing/mod_tests.rs': 'const X: &str = "https://api.tinyhumans.ai";\n',
  });
  try {
    assert.deepEqual(checkRepository(root), []);
    const run = spawnSync('node', [CHECKER, root], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stdout + run.stderr);
    assert.match(run.stdout, /ok/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('a hosted reference fails the checker with file and line', () => {
  const root = fixture({ 'a/mod.rs': 'fn x() {}\nconst U: &str = "https://api.tinyhumans.ai";\n' });
  try {
    const hits = checkRepository(root);
    assert.equal(hits.length, 1);
    assert.match(formatReport(hits), /a\/mod\.rs:2: tinyhumans\.ai/);
    const run = spawnSync('node', [CHECKER, root], { encoding: 'utf8' });
    assert.equal(run.status, 1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('a missing core tree exits 2 instead of passing vacuously', () => {
  const root = mkdtempSync(join(tmpdir(), 'core-hosted-free-empty-'));
  try {
    const run = spawnSync('node', [CHECKER, root], { encoding: 'utf8' });
    assert.equal(run.status, 2);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
