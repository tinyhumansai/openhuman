import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const runner = path.join(repoRoot, "scripts", "ci", "rust-coverage.sh");

function extractFunction(name) {
  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf(`${name}() {`);
  assert.notEqual(start, -1, `${name}() not found in ${runner}`);
  const end = source.indexOf("\n}\n", start);
  assert.notEqual(end, -1, `could not find the end of ${name}()`);
  return source.slice(start, end + 3);
}

function withRunnerFunctions(functions, preamble, body) {
  const script = [
    "set -euo pipefail",
    "exec 2>&1",
    preamble,
    ...functions.map(extractFunction),
    body,
  ].join("\n");
  try {
    return {
      status: 0,
      output: execFileSync("bash", ["-c", script], { encoding: "utf8" }),
    };
  } catch (error) {
    return {
      status: error.status,
      output: `${error.stdout ?? ""}${error.stderr ?? ""}`,
    };
  }
}

test("a failed raw coverage module is recorded and later modules still run", () => {
  const result = withRunnerFunctions(
    ["suite", "target_features_satisfied", "run_integration_target"],
    [
      'TEST_TARGET_REQS=""',
      'PRODUCT_FEATURES=""',
      "FAILED_SUITES=()",
      "log() { printf '%s\\n' \"$*\"; }",
      "raw_coverage_modules() { printf 'first\\nsecond\\n'; }",
      'llvm_cov() { for arg in "$@"; do case "$arg" in first::) return 7 ;; esac; done; echo RAN-LATER; }',
    ].join("\n"),
    'run_integration_target raw_coverage_all; printf "failed=%s\\n" "${FAILED_SUITES[@]}"',
  );

  assert.equal(result.status, 0, result.output);
  assert.match(result.output, /running raw coverage module: second/);
  assert.match(result.output, /RAN-LATER/);
  assert.match(result.output, /failed=raw_coverage_all::first \(exit 7\)/);
});

test("a cancelled raw coverage module stops the run at once", () => {
  const result = withRunnerFunctions(
    ["suite", "target_features_satisfied", "run_integration_target"],
    [
      'TEST_TARGET_REQS=""',
      'PRODUCT_FEATURES=""',
      "FAILED_SUITES=()",
      "log() { printf '%s\\n' \"$*\"; }",
      "raw_coverage_modules() { printf 'first\\nsecond\\n'; }",
      'llvm_cov() { for arg in "$@"; do case "$arg" in first::) return 143 ;; esac; done; echo RAN-LATER; }',
    ].join("\n"),
    "run_integration_target raw_coverage_all",
  );

  assert.equal(result.status, 143, result.output);
  assert.doesNotMatch(result.output, /running raw coverage module: second/);
  assert.doesNotMatch(result.output, /RAN-LATER/);
});

test("an integration target missing a required product feature is skipped", () => {
  const result = withRunnerFunctions(
    ["suite", "target_features_satisfied", "run_integration_target"],
    [
      "FAILED_SUITES=()",
      "TEST_TARGET_REQS=$'memory_artifacts_e2e\\tmemory-git'",
      'PRODUCT_FEATURES="channels,flows"',
      "log() { printf '%s\\n' \"$*\"; }",
      "llvm_cov() { echo RAN-TARGET; }",
    ].join("\n"),
    "run_integration_target memory_artifacts_e2e",
  );

  assert.equal(result.status, 0, result.output);
  assert.match(result.output, /skipping memory_artifacts_e2e/);
  assert.doesNotMatch(result.output, /RAN-TARGET/);
});

test("an integration target with all required product features runs", () => {
  const result = withRunnerFunctions(
    ["suite", "target_features_satisfied", "run_integration_target"],
    [
      "FAILED_SUITES=()",
      "TEST_TARGET_REQS=$'memory_artifacts_e2e\\tmemory-git'",
      'PRODUCT_FEATURES="memory-github,memory-git,flows"',
      "log() { printf '%s\\n' \"$*\"; }",
      "llvm_cov() { echo RAN-TARGET; }",
    ].join("\n"),
    "run_integration_target memory_artifacts_e2e",
  );

  assert.equal(result.status, 0, result.output);
  assert.match(result.output, /RAN-TARGET/);
});


test("headless Embed coverage gets an isolated disposable encryption key", () => {
  const result = withRunnerFunctions(
    ["llvm_cov_product_facade", "llvm_cov_embed"],
    [
      'PRODUCT_FEATURES="channels"',
      'export OPENHUMAN_KEYRING_MASTER_KEY=operator-key',
      'export OPENHUMAN_KEYRING_MASTER_KEY_FILE=operator-file',
      'bash() { [[ "${OPENHUMAN_KEYRING_MASTER_KEY:-}" =~ ^[0-9a-f]{64}$ ]] || return 9; [[ -z "${OPENHUMAN_KEYRING_MASTER_KEY_FILE:-}" ]] || return 10; echo DISPOSABLE-KEY; }',
    ].join("\n"),
    'llvm_cov_embed --no-report -p openhuman-embed; [[ "$OPENHUMAN_KEYRING_MASTER_KEY" == operator-key ]]; [[ "$OPENHUMAN_KEYRING_MASTER_KEY_FILE" == operator-file ]]; echo PARENT-UNCHANGED',
  );
  assert.equal(result.status, 0, result.output);
  assert.match(result.output, /DISPOSABLE-KEY/);
  assert.match(result.output, /PARENT-UNCHANGED/);
});
