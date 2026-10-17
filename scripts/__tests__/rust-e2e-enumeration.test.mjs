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

test("Rust E2E keeps module cache paths out of a shared runner home", (t) => {
  if (process.platform === "win32") return t.skip("runner shell test requires Unix permissions");

  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf('CREATED_XDG_DATA_HOME=""');
  const end = source.indexOf("\ncleanup() {", start);
  assert.notEqual(start, -1, "runner initializes its private XDG directories");
  assert.notEqual(end, -1, "runner XDG setup ends before cleanup");
  const setup = source.slice(start, end);
  const script = [
    "set -euo pipefail",
    setup,
    'printf "DATA=%s\\nCACHE=%s\\nCREATED_DATA=%s\\nCREATED_CACHE=%s\\n" "${XDG_DATA_HOME:-}" "${XDG_CACHE_HOME:-}" "$CREATED_XDG_DATA_HOME" "${CREATED_XDG_CACHE_HOME:-}"',
  ].join("\n");
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "rust-e2e-shared-home-"));
  fs.chmodSync(home, 0o755);
  let output;
  try {
    output = execFileSync("bash", ["-c", script], {
      encoding: "utf8",
      env: { ...process.env, HOME: home, XDG_DATA_HOME: "", XDG_CACHE_HOME: "" },
    });
    const values = Object.fromEntries(
      output.trim().split("\n").map((line) => line.split("=", 2)),
    );
    assert.ok(values.DATA, output);
    assert.ok(values.CACHE && values.CACHE !== values.DATA, output);
    assert.equal(values.CREATED_DATA, values.DATA, output);
    assert.equal(values.CREATED_CACHE, values.CACHE, output);
    assert.equal(fs.statSync(values.DATA).mode & 0o777, 0o700);
    assert.equal(fs.statSync(values.CACHE).mode & 0o777, 0o700);
    assert.ok(!values.DATA.startsWith(home), output);
  } finally {
    const createdData = output?.match(/^CREATED_DATA=(.*)$/m)?.[1];
    const createdCache = output?.match(/^CREATED_CACHE=(.*)$/m)?.[1];
    if (createdData) fs.rmSync(createdData, { recursive: true, force: true });
    if (createdCache) fs.rmSync(createdCache, { recursive: true, force: true });
    fs.rmSync(home, { recursive: true, force: true });
  }
});

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
function runSuite({ listStdout, listStatus, failTest = "", includeExtraArgs = true }) {
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
    `EXTRA_ARGS=(${includeExtraArgs ? "--nocapture" : ""})`,
    `HAS_EXTRA_ARGS=${includeExtraArgs ? 1 : 0}`,
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

const runOf = (name, includeExtraArgs = true) =>
  `test --manifest-path Cargo.toml --features voice --test json_rpc_e2e ${name} -- --exact --test-threads=1${includeExtraArgs ? " --nocapture" : ""}`;

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

test("a successful listing with no test rows fails the suite", () => {
  const res = runSuite({
    listStdout: "0 tests, 0 benchmarks\n",
    listStatus: 0,
  });

  assert.notEqual(res.status, 0, res.output);
  assert.doesNotMatch(res.output, /SUITE-REPORTED-SUCCESS/);
  assert.match(res.output, /enumeration returned no tests/);
  assert.deepEqual(res.runs, []);
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

test("an empty EXTRA_ARGS array is safe under Bash nounset", () => {
  const res = runSuite({
    listStdout: "scenario_a: test\n",
    listStatus: 0,
    includeExtraArgs: false,
  });

  assert.equal(res.status, 0, res.output);
  assert.deepEqual(res.runs, [runOf("scenario_a", false)]);
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

test("mcp_registry_e2e runs serially because its module state is process-global", () => {
  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf('if [ "$suite" = "mcp_registry_e2e" ]');
  assert.ok(start >= 0, "MCP registry target has an explicit runner branch");
  const end = source.indexOf('  if [ "$HAS_EXTRA_ARGS" -eq 1 ]', start);
  assert.ok(end > start, "MCP registry branch ends before the generic suite runner");
  assert.match(
    source.slice(start, end),
    /--test-threads=1/,
    "per-config module clients must not race through process-global state",
  );
});

test("the E2E runner stages released search and connector fixtures, and builds only in local source mode", () => {
  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf("# Module-backed Composio coverage");
  const end = source.indexOf('echo "[rust-e2e] Running', start);
  assert.ok(start >= 0 && end > start, "module preparation block exists");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-e2e-modules-"));
  try {
    const output = execFileSync(
      "bash",
      [
        "-c",
        [
          "set -euo pipefail",
          'REPO_ROOT="$STUB_DIR"',
          "e2e_source_ids=(tinyconnectors tinysearch)",
          "unset OPENHUMAN_MODULE_PATH",
          'stage_source_modules() { printf "built=%s\\n" "$*"; }',
          source.slice(start, end),
          "printf 'path=%s\\n' \"${OPENHUMAN_MODULE_PATH:-}\"",
        ].join("\n"),
      ],
      { encoding: "utf8", env: { ...process.env, STUB_DIR: dir } },
    );
    assert.match(output, /built=tinyconnectors tinysearch/);
    assert.ok(
      output.includes(
        `path=${dir}/target/module-sources/tinysearch:${dir}/target/module-sources/tinyconnectors`,
      ),
      output,
    );
    assert.doesNotMatch(source, /--target-dir/, "no temp or ad hoc target dir");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
