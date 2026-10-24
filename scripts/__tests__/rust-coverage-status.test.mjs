import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const runner = path.join(repoRoot, "scripts", "ci", "rust-coverage.sh");

function extractFunction(name, scriptPath = runner) {
  const source = fs.readFileSync(scriptPath, "utf8");
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
    ...functions.map((name) => extractFunction(name)),
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

test("headless Embed coverage forwards core features and preserves caller credentials", () => {
  const result = withRunnerFunctions(
    ["llvm_cov_product_facade", "llvm_cov_embed"],
    [
      'CORE_PRODUCT_FEATURES="channels"',
      "export OPENHUMAN_KEYRING_MASTER_KEY=operator-key",
      "export OPENHUMAN_KEYRING_MASTER_KEY_FILE=operator-file",
      'bash() { [[ "$OPENHUMAN_KEYRING_MASTER_KEY" == operator-key ]] || return 9; [[ "$OPENHUMAN_KEYRING_MASTER_KEY_FILE" == operator-file ]] || return 10; [[ "$*" == *"--features channels"* ]] || return 11; echo CORE-FEATURES; }',
    ].join("\n"),
    'llvm_cov_embed --no-report -p openhuman-embed; [[ "$OPENHUMAN_KEYRING_MASTER_KEY" == operator-key ]]; [[ "$OPENHUMAN_KEYRING_MASTER_KEY_FILE" == operator-file ]]; echo PARENT-UNCHANGED',
  );
  assert.equal(result.status, 0, result.output);
  assert.match(result.output, /CORE-FEATURES/);
  assert.match(result.output, /PARENT-UNCHANGED/);
});

for (const [label, scriptPath, invoke] of [
  [
    "mock runner",
    path.join(repoRoot, "scripts", "test-rust-with-mock.sh"),
    "cargo_test",
  ],
  ["coverage runner", runner, "llvm_cov"],
]) {
  test(`${label} selects the real isolated runtime-reaper regression`, () => {
    const script = [
      "set -euo pipefail",
      'suite() { shift; "$@"; }',
      `${invoke}() { printf '%s\\n' "$@"; }`,
      extractFunction("run_build_only_reaper_test", scriptPath),
      "run_build_only_reaper_test",
    ].join("\n");
    const args = execFileSync("bash", ["-c", script], { encoding: "utf8" })
      .trim()
      .split("\n");
    assert.ok(
      args.includes("--exact"),
      "the runtime singleton requires its own process",
    );
    assert.ok(
      args.includes(
        "agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked",
      ),
      `the libtest selector must reach the declared regression: ${args.join(" ")}`,
    );
  });
}

for (const helper of [
  "llvm_cov",
  "llvm_cov_package",
  "llvm_cov_product_facade",
]) {
  test(`${helper} selects nextest for private suites and rejects empty selections`, () => {
    const result = withRunnerFunctions(
      [helper],
      [
        "OH_COV_RUNNER=nextest",
        "CORE_PRODUCT_FEATURES=channels",
        'bash() { printf "%s\\n" "$*"; }',
      ].join("\n"),
      `${helper} --no-report -p openhuman-embed --all-targets`,
    );
    assert.equal(result.status, 0, result.output);
    assert.match(
      result.output,
      /cargo llvm-cov nextest --profile ci --no-tests=fail/,
    );
    assert.match(result.output, /--no-report -p openhuman-embed --all-targets/);
  });
}

test("private core coverage runs sequentially without a second target directory", () => {
  const source = fs.readFileSync(runner, "utf8");
  assert.doesNotMatch(source, /UNIT_PID|cov-unit|export CARGO_TARGET_DIR/);
  assert.match(source, /suite "openhuman product suites \(nextest\)" llvm_cov/);
  assert.match(
    source,
    /suite "openhuman-embed \+ openhuman-tinyhumans \(default\)" llvm_cov_package/,
  );
});

test("coverage finalizer retains interrupted reports and the cancellation status", () => {
  const result = withRunnerFunctions(
    ["preserve_coverage"],
    [
      "OUT=interrupted.info",
      "COVERAGE_REPORTED=0",
      'log() { printf "%s\\n" "$*"; }',
      'llvm_cov() { printf "REPORT %s\\n" "$*"; return 9; }',
    ].join("\n"),
    "trap preserve_coverage EXIT; exit 143",
  );
  assert.equal(result.status, 143, result.output);
  assert.match(
    result.output,
    /REPORT report --lcov --output-path interrupted.info/,
  );
});

function runMockedCoverage(exitCode) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-coverage-runner-"));
  try {
    for (const folder of [
      "bin",
      "scripts/ci",
      "crates/openhuman-cli",
      ".config",
      "tests",
    ])
      fs.mkdirSync(path.join(dir, folder), { recursive: true });
    const write = (name, content) =>
      fs.writeFileSync(path.join(dir, name), content, { mode: 0o755 });
    for (const name of [
      "scripts/ci/nextest-coverage.sh",
      "scripts/ci/nextest-report.mjs",
      ".config/nextest.toml",
    ])
      fs.copyFileSync(path.join(repoRoot, name), path.join(dir, name));
    write("scripts/ci/product-features.sh", "#!/bin/bash\necho channels\n");
    write("scripts/ci-cancel-aware.sh", '#!/bin/bash\nexec "$@"\n');
    write("scripts/ci/assert-coverage-presence.sh", "#!/bin/bash\nexit 0\n");
    write("crates/openhuman-cli/Cargo.toml", "");
    write("tests/active.rs", "#[test] fn active() {}\n");
    write("bin/rustup", "#!/bin/bash\nexit 1\n");
    write(
      "bin/cargo",
      `#!/bin/bash
printf '%s\\n' "$*" >> "$MOCK_CALLS"
case "$*" in *"llvm-cov nextest"*"-E package(=openhuman) | "*) [ "${exitCode}" = 0 ] || exit "${exitCode}" ;; esac
node - "$@" <<'JS'
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === 'metadata') console.log(JSON.stringify({ packages: [] }));
if (args[0] === 'llvm-cov' && args[1] === 'show-env') console.log('export CARGO_LLVM_COV_TARGET_DIR=target');
if (args[0] === 'nextest' && args[1] === 'list') {
  console.log(JSON.stringify({ 'rust-suites': { active: { 'binary-name': 'active', testcases: { active: { ignored: false, 'filter-match': { status: 'matches' } } } } } }));
}
const pos = args.indexOf('--config-file');
if (pos >= 0) {
  const config = fs.readFileSync(args[pos + 1], 'utf8');
  const report = JSON.parse(config.split('\\n').find(line => line.startsWith('path = ')).slice(7));
  fs.writeFileSync(report, '<testsuites><testsuite name="openhuman-cli::active" tests="1"></testsuite></testsuites>');
}
JS
`,
    );
    let status = 0;
    try {
      execFileSync("bash", [runner], {
        cwd: dir,
        env: {
          ...process.env,
          PATH: `${dir}/bin:${process.env.PATH}`,
          MOCK_CALLS: `${dir}/calls`,
          OH_COV_RUNNER: "nextest",
          OH_COV_DOCTESTS: "0",
        },
        stdio: "pipe",
      });
    } catch (error) {
      status = error.status;
    }
    return {
      status,
      calls: fs.readFileSync(`${dir}/calls`, "utf8").trim().split("\n"),
    };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("private coverage executes every package configuration once and reports after a failed suite", () => {
  const { status, calls } = runMockedCoverage(7);
  assert.equal(status, 1);
  const selections = calls.filter((line) => line.includes("llvm-cov nextest"));
  // The product suites (core, embed, tinyhumans, cli), rpc, the default
  // feature suite (embed and tinyhumans together) and the tui.
  assert.equal(selections.length, 4);
  assert.equal(new Set(selections).size, 4);
  assert.equal(
    selections.filter((line) =>
      line.includes("-p openhuman-embed -p openhuman-tinyhumans --all-targets") &&
      !line.includes("-p openhuman-cli"),
    ).length,
    1,
  );
  assert.ok(selections.every((line) => line.includes("--no-tests=fail")));
  // One build and one run of the product graph, over the union of the old
  // per-package filters, listed first with the same arguments.
  const product = selections.filter((line) =>
    line.includes(
      "--features channels --no-report --no-fail-fast -p openhuman -p openhuman-embed -p openhuman-tinyhumans -p openhuman-cli --all-targets -E ",
    ),
  );
  assert.equal(product.length, 1);
  assert.equal(
    product[0].slice(product[0].indexOf("-E ") + 3),
    "package(=openhuman) | package(=openhuman-embed) | package(=openhuman-tinyhumans) | (package(=openhuman-cli) & kind(test))",
  );
  const listing = calls.findIndex((line) => line.startsWith("nextest list"));
  assert.ok(listing >= 0 && listing < calls.indexOf(product[0]));
  assert.equal(
    calls.filter((line) => line.includes("llvm-cov report")).length,
    1,
  );
});

test("private coverage cancels remaining suites but still preserves a report", () => {
  const { status, calls } = runMockedCoverage(143);
  assert.equal(status, 143);
  assert.equal(
    calls.filter((line) => line.includes("llvm-cov nextest")).length,
    1,
  );
  assert.equal(
    calls.filter((line) => line.includes("llvm-cov report")).length,
    1,
  );
});

test("private coverage passes when all four selected configurations report tests", () => {
  const { status } = runMockedCoverage(0);
  assert.equal(status, 0);
});

test("instrumented CLI inventory is cancellation-aware and reuses llvm-cov's environment", () => {
  const source = fs.readFileSync(runner, "utf8");
  assert.match(
    source,
    /bash scripts\/ci-cancel-aware\.sh cargo llvm-cov show-env --sh/,
  );
  assert.match(source, /eval "\$\{inventory_env\}"/);
  assert.match(
    source,
    /bash scripts\/ci-cancel-aware\.sh cargo nextest list --message-format json/,
  );
});

test("product inventory lists llvm-cov's own build instead of instrumenting a second copy", () => {
  const source = fs.readFileSync(runner, "utf8");
  const start = source.indexOf("  product_inventory() (");
  const end = source.indexOf("\n  )", start);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  const inventory = source.slice(start, end + "\n  )".length);
  const script = [
    "set -euo pipefail",
    'CORE_PRODUCT_FEATURES="channels,modules"',
    ...source
      .split("\n")
      .filter((line) => /^(UNIFIED_PRODUCT_ARGS|FILTER_[A-Z_]+)=/.test(line)),
    "expected_inventory=/dev/null",
    "export CARGO_BUILD_TARGET_DIR=preserved-target",
    `bash() {
      if [[ "$*" == *"llvm-cov show-env"* ]]; then
        for arg in "$@"; do
          [[ "$arg" != --features ]] || { echo 'show-env rejects --features' >&2; return 64; }
        done
        printf '%s\\n' 'export INVENTORY_INSTRUMENTED=1' 'export CARGO_TARGET_DIR=instrumented-target' 'export CARGO_LLVM_COV_TARGET_DIR=cov-target'
      elif [[ "$*" == *"nextest list"* ]]; then
        [[ "$*" == *"--features channels,modules --target-dir cov-target/llvm-cov-target -p openhuman -p openhuman-embed -p openhuman-tinyhumans -p openhuman-cli --all-targets -E package(=openhuman) | package(=openhuman-embed) | package(=openhuman-tinyhumans) | (package(=openhuman-cli) & kind(test))"* ]] || return 65
        [[ "$INVENTORY_INSTRUMENTED" == 1 && "$CARGO_TARGET_DIR" == instrumented-target && "$CARGO_BUILD_TARGET_DIR" == preserved-target ]] || return 66
        [[ "$LLVM_PROFILE_FILE" == */ci-out/nextest/product-inventory-profraw/* ]] || return 68
      else return 67
      fi
    }`,
    inventory,
    "product_inventory",
  ].join("\n");
  execFileSync("bash", ["-c", script], { encoding: "utf8" });
});

for (const platform of ["Linux", "Darwin", "MINGW64_NT-10.0", "Windows_NT"]) {
  test(`coverage inventory applies only documented target exceptions on ${platform}`, () => {
    const windows = platform.startsWith("MINGW") || platform === "Windows_NT";
    const result = withRunnerFunctions(
      ["target_expected_in_coverage"],
      'log() { printf "%s\\n" "$*"; }',
      [
        `if target_expected_in_coverage composio_list_tools_stack_overflow_regression '${platform}'; then exit 11; fi`,
        `if target_expected_in_coverage windows_sandbox_env_e2e '${platform}'; then ${windows ? ":" : "exit 12"}; else ${windows ? "exit 13" : ":"}; fi`,
        `target_expected_in_coverage json_rpc_threads_e2e '${platform}'`,
        `target_expected_in_coverage future_empty_suite '${platform}'`,
      ].join("\n"),
    );
    assert.equal(result.status, 0, result.output);
    assert.match(result.output, /cfg\(not\(coverage\)\)/);
  });
}

test("coverage inventory exceptions match the source gates and remain required at selection", () => {
  assert.match(
    fs.readFileSync(
      path.join(
        repoRoot,
        "tests/composio_list_tools_stack_overflow_regression.rs",
      ),
      "utf8",
    ),
    /#!\[cfg\(not\(coverage\)\)\]/,
  );
  const windows = fs.readFileSync(
    path.join(repoRoot, "tests/windows_sandbox_env_e2e.rs"),
    "utf8",
  );
  const functions = [
    ...windows.matchAll(
      /#\[cfg\(windows\)\]\s*#\[(?:tokio::)?test(?:\([^\n]*\))?\]/g,
    ),
  ];
  assert.ok(
    functions.length > 0,
    "Windows-only fixture must retain gated tests",
  );
  assert.equal(
    functions.length,
    [...windows.matchAll(/#\[(?:tokio::)?test(?:\([^\n]*\))?\]/g)].length,
    "every Windows fixture test must remain gated",
  );
  assert.match(
    fs.readFileSync(runner, "utf8"),
    /target_features_satisfied "\$\{target\}" && target_expected_in_coverage "\$\{target\}"/,
  );
});

test("nextest product suites are one build and one run over the union of their filters", () => {
  // Join line continuations so each suite call reads as one command.
  const source = fs.readFileSync(runner, "utf8").replace(/\\\n\s*/g, "");
  assert.match(
    source,
    /suite "openhuman product suites \(nextest\)" llvm_cov --no-report --no-fail-fast "\$\{UNIFIED_PRODUCT_ARGS\[@\]\}" -E "\$\{FILTER_PRODUCT\}"/,
  );
  // The inventory is listed with the same arguments so the run reuses its build.
  assert.match(
    source,
    /cargo nextest list .*"\$\{UNIFIED_PRODUCT_ARGS\[@\]\}" -E "\$\{FILTER_PRODUCT\}"/,
  );
  // One package set, whose cargo resolution unifies the core's features once.
  const args = source.match(/^UNIFIED_PRODUCT_ARGS=\((.*)\)$/m)[1];
  assert.equal(
    args,
    "-p openhuman -p openhuman-embed -p openhuman-tinyhumans -p openhuman-cli --all-targets",
  );
  // Exact package matches: `package(openhuman)` alone would be a glob of the name.
  for (const value of [
    "FILTER_CORE='package(=openhuman)'",
    "FILTER_EMBED='package(=openhuman-embed)'",
    "FILTER_TINYHUMANS='package(=openhuman-tinyhumans)'",
    "FILTER_CLI_TESTS='package(=openhuman-cli) & kind(test)'",
    'FILTER_PRODUCT="${FILTER_CORE} | ${FILTER_EMBED} | ${FILTER_TINYHUMANS} | (${FILTER_CLI_TESTS})"',
  ])
    assert.ok(source.includes(value), value);
  // No per-package product suites remain; rpc, tui and the default feature
  // suites keep their own resolutions.
  assert.doesNotMatch(source, /-p openhuman-cli --tests/);
  assert.doesNotMatch(source, /-E "\$\{FILTER_(CORE|EMBED|TINYHUMANS|CLI_TESTS)\}"/);

  assert.match(source, /suite "openhuman-rpc" llvm_cov_package .*-p openhuman-rpc --all-targets/);
  assert.match(source, /suite "openhuman-tui" llvm_cov_package .*-p openhuman-tui --all-targets/);
  assert.match(
    source,
    /suite "openhuman-embed \+ openhuman-tinyhumans \(default\)" llvm_cov_package .*-p openhuman-embed -p openhuman-tinyhumans --all-targets/,
  );
});

test("coverage suites record why cargo rebuilt and drop the verbose noise", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-coverage-rebuilds-"));
  try {
    const result = withRunnerFunctions(
      ["explain_rebuilds"],
      "",
      [
        `printf '%s\\n' '       Dirty libc v0.2.189: the file \`build.rs\` has changed' '     Running \`rustc --crate-name libc\`' '       Fresh memchr v2.7.4' '   Compiling libc v0.2.189' 'test output' | explain_rebuilds ${JSON.stringify(path.join(dir, "suite.log"))}`,
      ].join("\n"),
    );
    assert.equal(result.status, 0, result.output);
    assert.match(result.output, /Dirty libc/);
    assert.match(result.output, /Compiling libc/);
    assert.match(result.output, /test output/);
    assert.doesNotMatch(result.output, /Running `rustc|Fresh memchr/);
    assert.equal(
      fs.readFileSync(path.join(dir, "suite.log"), "utf8"),
      "       Dirty libc v0.2.189: the file `build.rs` has changed\n",
    );
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the nextest coverage run scales the model retry backoff down, overridably", () => {
  const source = fs.readFileSync(runner, "utf8");
  const block = source.slice(source.indexOf('if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then\n  # One process per test'));
  assert.match(
    block.split("\nfi\n")[0],
    /export OPENHUMAN_MODEL_RETRY_BACKOFF_MS="\$\{OPENHUMAN_MODEL_RETRY_BACKOFF_MS:-100\}"/,
  );
});
