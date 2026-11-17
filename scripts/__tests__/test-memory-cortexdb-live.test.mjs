import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

test('live CortexDB harness runs the ignored test against its local server URL', async () => {
  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'cortexdb-live-runner-'));
  const binDir = path.join(tempDir, 'bin');
  const recordsDir = path.join(tempDir, 'records');
  await mkdir(binDir);
  await mkdir(recordsDir);

  const writeStub = async (name, contents) => {
    const file = path.join(binDir, name);
    await writeFile(file, contents, 'utf8');
    await chmod(file, 0o755);
  };

  try {
    await writeStub('docker', '#!/bin/sh\nexit 0\n');
    await writeStub(
      'curl',
      `#!/bin/sh
case "$*" in
  *"/v1/admin/health"*)
    if [ ! -e "${recordsDir}/probed" ]; then
      : > "${recordsDir}/probed"
      exit 7
    fi
    printf '{"status":"ready"}'
    ;;
  *"/v1/admin/ready"*) printf 'ready' ;;
  *) exit 1 ;;
esac
`,
    );
    await writeStub(
      'cargo',
      `#!/bin/sh
printf '%s\\n' "$@" > "$CORTEXDB_TEST_ARGS"
printf '%s' "$OPENHUMAN_LIVE_CORTEXDB_URL" > "$CORTEXDB_TEST_URL"
`,
    );

    const script = fileURLToPath(
      new URL('../test-memory-cortexdb-live.sh', import.meta.url),
    );
    const port = '43142';
    const result = spawnSync('bash', [script], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: `${binDir}:${process.env.PATH}`,
        CORTEXDB_PORT: port,
        CORTEXDB_TEST_ARGS: path.join(recordsDir, 'cargo-args'),
        CORTEXDB_TEST_URL: path.join(recordsDir, 'cargo-url'),
      },
      timeout: 10_000,
    });

    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const cargoArgs = (await readFile(path.join(recordsDir, 'cargo-args'), 'utf8'))
      .trim()
      .split('\n');
    const separator = cargoArgs.indexOf('--');
    assert.notEqual(separator, -1, 'cargo test should receive test-harness arguments');
    assert.ok(cargoArgs.slice(0, separator).includes('--test'));
    assert.equal(cargoArgs[separator + 1], '--ignored');
    assert.equal(cargoArgs[separator + 2], '--nocapture');
    assert.equal(
      await readFile(path.join(recordsDir, 'cargo-url'), 'utf8'),
      `http://127.0.0.1:${port}`,
    );
  } finally {
    await rm(tempDir, { recursive: true, force: true });
  }
});
