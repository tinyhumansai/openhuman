import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(
  new URL("../ci/security-native-fixture.sh", import.meta.url),
);
const workflow = fileURLToPath(
  new URL("../../.github/workflows/security-native.yml", import.meta.url),
);

// Run the fixture script with a `bash` shim first on PATH that records the
// cargo invocation it would have launched, instead of building the core.
function run(env) {
  const dir = mkdtempSync(join(tmpdir(), "security-native-fixture-"));
  const log = join(dir, "invocation.txt");
  const shim = join(dir, "bash");
  writeFileSync(
    shim,
    `#!/bin/sh\n{ echo "ARGS $*"; echo "BUDGET=$OPENHUMAN_SECURITY_PERF_BUDGET"; echo "OUT=$OPENHUMAN_SECURITY_PERF_OUT"; } >> "${log}"\n`,
  );
  chmodSync(shim, 0o755);
  const result = spawnSync("/bin/bash", [script], {
    encoding: "utf8",
    env: {
      PATH: `${dir}:${process.env.PATH}`,
      HOME: dir,
      OPENHUMAN_TEST_SECURITY_MODULE: "/nonexistent/libtinysecurity_module.so",
      ...env,
    },
  });
  return { result, log: readFileSync(log, "utf8"), dir };
}

test("default run exercises the native tests in the debug profile", () => {
  const { result, log } = run({});
  assert.equal(result.status, 0, result.stderr);
  assert.match(log, /cargo test -p openhuman --lib --no-default-features/);
  assert.doesNotMatch(log, /--release/);
  assert.match(log, /BUDGET=\n/);
});

test("budget mode runs under --release and names the perf record output", () => {
  const { result, log, dir } = run({ OPENHUMAN_SECURITY_PERF_BUDGET: "1" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(log, /--lib --release --no-default-features/);
  assert.match(log, /BUDGET=1\n/);
  assert.match(log, /OUT=.*target\/security-perf\.json\n/);
  assert.ok(dir);
});

test("a caller-provided perf output path is kept", () => {
  const { log } = run({
    OPENHUMAN_SECURITY_PERF_BUDGET: "1",
    OPENHUMAN_SECURITY_PERF_OUT: "/tmp/custom-perf.json",
  });
  assert.match(log, /OUT=\/tmp\/custom-perf\.json\n/);
});

test("workflow gates the budget in its own job and keeps the record", () => {
  const text = readFileSync(workflow, "utf8");
  assert.match(text, /latency-budget:/);
  assert.match(text, /OPENHUMAN_SECURITY_PERF_BUDGET: "1"/);
  assert.match(text, /name: security-perf/);
});
