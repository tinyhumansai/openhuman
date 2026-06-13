import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'merge-main-into-open-prs.mjs');

function writeFailingStub(binDir, name) {
  fs.writeFileSync(
    join(binDir, name),
    `#!/usr/bin/env sh\necho "${name} should not run for help" >&2\nexit 99\n`,
    { mode: 0o755 },
  );
}

function runHelp(flag) {
  const binDir = fs.mkdtempSync(join(os.tmpdir(), 'openhuman-git-gh-stub-'));
  writeFailingStub(binDir, 'git');
  writeFailingStub(binDir, 'gh');

  return spawnSync(process.execPath, [SCRIPT, flag], {
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${binDir}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH ?? ''}`,
    },
  });
}

test('merge-main-into-open-prs help exits before invoking git or gh', () => {
  for (const flag of ['--help', '-h']) {
    const result = runHelp(flag);

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /Usage: merge-main-into-open-prs\.mjs \[options\]/);
    assert.match(result.stdout, /-h, --help\s+Show this message\./);
    assert.equal(result.stderr, '');
  }
});
