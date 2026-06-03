import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'cancel-stale-pr-ci.mjs');

function runHelp(flag) {
  const binDir = fs.mkdtempSync(join(os.tmpdir(), 'openhuman-gh-stub-'));
  fs.writeFileSync(
    join(binDir, 'gh'),
    '#!/usr/bin/env sh\necho "gh should not run for help" >&2\nexit 99\n',
    { mode: 0o755 },
  );

  return spawnSync(process.execPath, [SCRIPT, flag], {
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${binDir}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH ?? ''}`,
    },
  });
}

test('cancel-stale-pr-ci help exits before invoking gh', () => {
  for (const flag of ['--help', '-h']) {
    const result = runHelp(flag);

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /Usage: cancel-stale-pr-ci\.mjs \[options\]/);
    assert.match(result.stdout, /-h, --help\s+Show this message\./);
    assert.equal(result.stderr, '');
  }
});
