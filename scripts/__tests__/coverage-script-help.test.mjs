import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPTS = resolve(HERE, '..');

function run(scriptName, args) {
  return spawnSync(process.execPath, [resolve(SCRIPTS, scriptName), ...args], {
    encoding: 'utf8',
  });
}

test('coverage helper scripts print help without running checks', () => {
  for (const [scriptName, usage] of [
    ['check-coverage-matrix.mjs', 'Usage: node scripts/check-coverage-matrix.mjs'],
    ['check-domain-e2e-coverage.mjs', 'Usage: node scripts/check-domain-e2e-coverage.mjs'],
  ]) {
    for (const helpFlag of ['--help', '-h']) {
      const result = run(scriptName, [helpFlag]);

      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout.trim(), usage);
      assert.equal(result.stderr, '');
    }
  }
});

test('coverage helper scripts reject unknown arguments', () => {
  for (const [scriptName, error] of [
    ['check-coverage-matrix.mjs', /check-coverage-matrix: unknown argument: --bogus/],
    ['check-domain-e2e-coverage.mjs', /check-domain-e2e-coverage: unknown argument: --bogus/],
  ]) {
    const result = run(scriptName, ['--bogus']);

    assert.equal(result.status, 2, result.stdout);
    assert.match(result.stderr, error);
  }
});
