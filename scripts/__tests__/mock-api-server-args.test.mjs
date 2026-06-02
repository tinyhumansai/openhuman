import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'mock-api-server.mjs');

function run(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: 'utf8',
  });
}

test('mock-api-server --help prints usage without opening a listener', () => {
  const result = run(['--help']);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Usage: node scripts\/mock-api-server\.mjs \[--port <port>\]/);
  assert.equal(result.stderr, '');
});

test('mock-api-server rejects invalid explicit ports before startup', () => {
  for (const args of [['--port', 'nope'], ['--port', '65536'], ['--port'], ['-p', '--help']]) {
    const result = run(args);

    assert.equal(result.status, 2, result.stdout);
    assert.match(result.stderr, /--port .*integer between 1 and 65535/);
    assert.doesNotMatch(result.stderr, /Failed to start/);
  }
});
