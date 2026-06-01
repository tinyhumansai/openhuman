import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'verify-i18n-bundle.mjs');

function run(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: 'utf8',
  });
}

test('verify-i18n-bundle --dist rejects a missing path before filesystem checks', () => {
  const result = run(['--dist']);

  assert.equal(result.status, 2, result.stderr);
  assert.match(result.stderr, /--dist requires a path/);
  assert.doesNotMatch(result.stderr, /dist directory does not exist/);
});

test('verify-i18n-bundle --dist rejects another flag as the path value', () => {
  const result = run(['--dist', '--help']);

  assert.equal(result.status, 2, result.stderr);
  assert.match(result.stderr, /--dist requires a path/);
  assert.doesNotMatch(result.stderr, /dist directory does not exist/);
});
