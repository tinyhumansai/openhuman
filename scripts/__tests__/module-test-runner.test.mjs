import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const runner = path.join(repoRoot, "scripts", "ci", "run-module-gated-tests.sh");

test("runs only the source-marked module tests in isolated processes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-module-tests-"));
  const bin = path.join(root, "bin");
  const log = path.join(root, "cargo.log");
  fs.mkdirSync(bin);
  fs.writeFileSync(
    path.join(bin, "node"),
    `#!/usr/bin/env bash
if [[ "$*" == *"--skipped"* ]]; then exit 0; fi
printf '%s\n' one two
`,
    { mode: 0o755 },
  );
  fs.writeFileSync(
    path.join(bin, "cargo"),
    `#!/usr/bin/env bash
printf '%s\n' "$*" >> "${log}"
if [[ "$*" == *"--list"* ]]; then
  printf '%s\n' 'openhuman::memory::tests::one: test' 'openhuman::memory::tests::two: test'
fi
`,
    { mode: 0o755 },
  );

  const result = spawnSync(runner, { cwd: repoRoot, env: { ...process.env, PATH: `${bin}:${process.env.PATH}` }, encoding: "utf8" });
  const calls = fs.readFileSync(log, "utf8");
  fs.rmSync(root, { recursive: true, force: true });

  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stdout, /scheduled: 2/);
  assert.match(result.stdout, /skipped: 0/);
  assert.match(calls, /--ignored --list/);
  assert.match(calls, /openhuman::memory::tests::one -- --ignored --exact --test-threads=1/);
  assert.match(calls, /openhuman::memory::tests::two -- --ignored --exact --test-threads=1/);
});

test("rejects a vacuous ignored-test inventory", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-module-tests-empty-"));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  fs.writeFileSync(path.join(bin, "node"), "#!/usr/bin/env bash\nexit 0\n", { mode: 0o755 });
  fs.writeFileSync(path.join(bin, "cargo"), "#!/usr/bin/env bash\nexit 0\n", { mode: 0o755 });

  const result = spawnSync(runner, { cwd: repoRoot, env: { ...process.env, PATH: `${bin}:${process.env.PATH}` }, encoding: "utf8" });
  fs.rmSync(root, { recursive: true, force: true });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /inventory is empty/);
});
