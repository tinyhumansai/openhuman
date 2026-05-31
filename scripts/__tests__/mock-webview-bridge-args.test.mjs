import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'mock-webview-bridge.mjs');

function run(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: 'utf8',
  });
}

test('mock-webview-bridge --help prints usage without opening a listener', () => {
  const result = run(['--help']);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Usage: node scripts\/mock-webview-bridge\.mjs \[port\]/);
  assert.equal(result.stderr, '');
});

test('mock-webview-bridge rejects invalid ports before WebSocket startup', () => {
  for (const args of [['nope'], ['0'], ['65536'], ['9826', 'extra'], ['9826', '']]) {
    const result = run(args);

    assert.equal(result.status, 2, result.stdout);
    assert.doesNotMatch(result.stderr, /ERR_SOCKET_BAD_PORT/);
  }
});
