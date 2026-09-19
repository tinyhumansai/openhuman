// Regression tests for test enumeration in scripts/test-rust-e2e.sh (#6236).
//
// `run_json_rpc_e2e_suite` lists the json_rpc_e2e cases with
// `cargo test -- --list`, then runs each in a fresh process. The listing used to
// be read through a process substitution, whose exit status nothing checks:
// when the target failed to compile, cargo listed nothing, the loop ran zero
// times, and `--suite json_rpc_e2e` exited 0 without executing a test. A
// listing that failed part-way ran whatever names it had printed, and also
// passed.
//
// Like coverage-runner-status.test.mjs, these evaluate the REAL function body,
// extracted from the script at test time, rather than a transcription of it — a
// copy would keep passing after the original regressed.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const runner = path.join(repoRoot, "scripts", "test-rust-e2e.sh");

/** Extract a top-level `name() { … }` block from the runner, verbatim. */
function extractFunction(name) {
  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf(`${name}() {`);
  assert.notEqual(
    start,
    -1,
    `${name}() not found in ${runner} — did it get renamed?`,
  );
  const end = source.indexOf("\n}\n", start);
  assert.notEqual(end, -1, `could not find the end of ${name}()`);
  return source.slice(start, end + 3);
}

// Stand-in for cargo. Behaviour comes from the environment rather than from
// text spliced into the script, so no case needs shell quoting.
//
// `--list` prints STUB_LIST_STDOUT and exits STUB_LIST_STATUS, with a compiler
// diagnostic on stderr when that status is non-zero. Any other invocation is a
// test run: its arguments are appended to STUB_RUN_LOG, one run per line, and
// it fails if they name STUB_FAIL_TEST.
const CARGO_STUB = `#!/usr/bin/env bash
for arg in "$@"; do
  if [ "$arg" = "--list" ]; then
    printf '%s' "$STUB_LIST_STDOUT"
    if [ "$STUB_LIST_STATUS" -ne 0 ]; then
      echo 'error: could not compile \`json_rpc_e2e\` (test "json_rpc_e2e")' >&2
    fi
    exit "$STUB_LIST_STATUS"
  fi
done
printf '%s\\n' "$*" >> "$STUB_RUN_LOG"
case " $* " in
  *" $STUB_FAIL_TEST "*) exit 101 ;;
esac
`;

// ci-cancel-aware.sh reduced to its contract with this function: run the
// command it is handed. Its signal handling is not what is under test.
const CANCEL_AWARE_STUB = `#!/usr/bin/env bash
exec "$@"
`;

/**
 * Run the real `run_json_rpc_e2e_suite` against a stubbed cargo.
 *
 * @param {object} opts
 * @param {string} opts.listStdout what `cargo test -- --list` prints
 * @param {number} opts.listStatus what it exits with
 * @param {string} [opts.failTest] a case whose run should fail
 */
function runSuite({ listStdout, listStatus, failTest = "" }) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-e2e-enumeration-"));
  const runLog = path.join(dir, "runs.log");
  fs.writeFileSync(path.join(dir, "cargo"), CARGO_STUB, { mode: 0o755 });
  fs.writeFileSync(path.join(dir, "ci-cancel-aware.sh"), CANCEL_AWARE_STUB, {
    mode: 0o755,
  });

  const script = [
    "set -euo pipefail",
    // Merge stderr into stdout so the command-not-found guard below sees
    // everything, on the success path as well as the failure path.
    "exec 2>&1",
    'SCRIPT_DIR="$STUB_DIR"',
    'CARGO_BIN="$STUB_DIR/cargo"',
    'PRODUCT_FEATURES="voice"',
    // Non-empty on purpose: it proves EXTRA_ARGS still reaches every case, and
    // bash before 4.4 (macOS /bin/bash is 3.2) treats an empty "${a[@]}" as
    // unbound under `set -u`, which would fail these tests locally for a
    // reason unrelated to enumeration.
    "EXTRA_ARGS=(--nocapture)",
    extractFunction("run_json_rpc_e2e_suite"),
    // A plain statement, exactly as the runner's suite loop calls it. Inside an
    // `if` condition bash suspends errexit for the whole function body, which
    // would change what a failing case does and test a different program.
    "run_json_rpc_e2e_suite",
    "echo SUITE-REPORTED-SUCCESS",
  ].join("\n");

  const env = {
    ...process.env,
    STUB_DIR: dir,
    STUB_RUN_LOG: runLog,
    STUB_LIST_STDOUT: listStdout,
    STUB_LIST_STATUS: String(listStatus),
    STUB_FAIL_TEST: failTest,
  };

  let result;
  try {
    result = {
      status: 0,
      output: execFileSync("bash", ["-c", script], { encoding: "utf8", env }),
    };
  } catch (err) {
    result = {
      status: err.status,
      output: `${err.stdout ?? ""}${err.stderr ?? ""}`,
    };
  }

  const runs = fs.existsSync(runLog)
    ? fs.readFileSync(runLog, "utf8").split("\n").filter(Boolean)
    : [];
  fs.rmSync(dir, { recursive: true, force: true });

  // A stub that failed to resolve would exit 127 and look like the failure a
  // test expects. Surface it as itself instead.
  assert.doesNotMatch(
    result.output,
    /command not found|No such file or directory/,
    `a stub did not resolve, so this result says nothing about enumeration:\n${result.output}`,
  );
  return { ...result, runs };
}

const runOf = (name) =>
  `test --manifest-path Cargo.toml --features voice --test json_rpc_e2e ${name} -- --exact --test-threads=1 --nocapture`;

test("a listing that fails before printing any name fails the suite", () => {
  // The case from #6236: the target does not compile, so there is nothing to
  // enumerate and nothing to run — which used to read as a clean pass.
  const res = runSuite({ listStdout: "", listStatus: 101 });

  assert.notEqual(
    res.status,
    0,
    `an enumeration failure must fail the suite:\n${res.output}`,
  );
  assert.doesNotMatch(res.output, /SUITE-REPORTED-SUCCESS/);
  assert.match(res.output, /could not enumerate json_rpc_e2e tests/);
  assert.deepEqual(res.runs, []);
});

test("a listing that fails part-way runs none of the names it printed", () => {
  // Fail-closed, not best-effort: the names seen before the failure are not
  // known to be the whole suite, so running them would pass on a subset.
  const res = runSuite({
    listStdout: "scenario_a: test\n",
    listStatus: 101,
  });

  assert.notEqual(res.status, 0, res.output);
  assert.doesNotMatch(res.output, /SUITE-REPORTED-SUCCESS/);
  assert.deepEqual(
    res.runs,
    [],
    "a partial listing must not be executed as though it were complete",
  );
});

test("a successful listing runs each case once, in its own process, with EXTRA_ARGS", () => {
  // libtest's real `--list` shape: `name: test` rows, a blank line, a summary.
  // Only the `: test` rows are cases.
  const res = runSuite({
    listStdout: "scenario_a: test\nscenario_b: test\n\n2 tests, 0 benchmarks\n",
    listStatus: 0,
  });

  assert.equal(res.status, 0, res.output);
  assert.match(res.output, /SUITE-REPORTED-SUCCESS/);
  assert.deepEqual(res.runs, [runOf("scenario_a"), runOf("scenario_b")]);
});

test("a failing case still fails the suite, and nothing after it runs", () => {
  // Pins behaviour the fix must not lose. The loop now reads a here-string
  // instead of a process substitution; a failing case must still stop it.
  const res = runSuite({
    listStdout: "scenario_a: test\nscenario_b: test\n",
    listStatus: 0,
    failTest: "scenario_a",
  });

  assert.notEqual(res.status, 0, res.output);
  assert.doesNotMatch(res.output, /SUITE-REPORTED-SUCCESS/);
  assert.deepEqual(res.runs, [runOf("scenario_a")]);
});
