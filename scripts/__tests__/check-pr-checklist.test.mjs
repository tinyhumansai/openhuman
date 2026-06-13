import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'check-pr-checklist.mjs');

function run(args, options = {}) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: 'utf8',
    ...options,
  });
}

test('check-pr-checklist --help prints usage and exits 0', () => {
  const result = run(['--help']);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Usage: check-pr-checklist\.mjs \[body-file\|-\]/);
  assert.equal(result.stderr, '');
});

test('check-pr-checklist -h prints usage and exits 0', () => {
  const result = run(['-h']);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Usage: check-pr-checklist\.mjs \[body-file\|-\]/);
  assert.equal(result.stderr, '');
});
