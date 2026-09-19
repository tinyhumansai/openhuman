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

test("a failed raw coverage module fails before later modules run", () => {
  const result = withRunnerFunctions(
    ["target_features_satisfied", "run_integration_target"],
    [
      'TEST_TARGET_REQS=""',
      'PRODUCT_FEATURES=""',
      "log() { printf '%s\\n' \"$*\"; }",
      "raw_coverage_modules() { printf 'first\\nsecond\\n'; }",
      'llvm_cov() { for arg in "$@"; do case "$arg" in first::) return 7 ;; esac; done; echo RAN-LATER; }',
    ].join("\n"),
    "run_integration_target raw_coverage_all",
  );

  assert.equal(result.status, 7, result.output);
  assert.doesNotMatch(result.output, /running raw coverage module: second/);
  assert.doesNotMatch(result.output, /RAN-LATER/);
});

test("an integration target missing a required product feature is skipped", () => {
  const result = withRunnerFunctions(
    ["target_features_satisfied", "run_integration_target"],
    [
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
    ["target_features_satisfied", "run_integration_target"],
    [
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
