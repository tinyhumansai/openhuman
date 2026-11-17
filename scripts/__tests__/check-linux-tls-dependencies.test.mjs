// Regression tests for scripts/check-linux-tls-dependencies.sh (#6530).
//
// The script used `mapfile`, a bash 4 builtin. The stock macOS /bin/bash is
// 3.2, where the run died partway with `mapfile: command not found` and exit
// 127, which reads like a policy failure. The script now collects the
// reqwest 0.13 versions with a `while read` loop that every bash runs.
//
// These run the REAL script with `cargo` replaced by a stub on PATH that
// answers `cargo tree` from fixture text, so no Rust build or network is
// needed. On a host whose /bin/bash is 3.2 every case also runs under it.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
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
const script = path.join(
  repoRoot,
  "scripts",
  "check-linux-tls-dependencies.sh",
);

const CLEAN_TREE = [
  "openhuman v0.1.0",
  "reqwest v0.12.9",
  "tokio v1.40.0",
].join("\n");

// Cargo 1.90 `cargo tree --prefix none --invert sentry` emits unindented
// package lines, for example `sentry v0.47.0` followed by
// `openhuman v0.64.4 (/checkout/crates/openhuman-core)`. The path below is
// normalized; the line shape and column-zero owner are taken from Cargo.
const SENTRY_OWNER_TREE = [
  "native-tls v0.2.14",
  "sentry v0.47.0",
  "openhuman v0.64.4 (/checkout/crates/openhuman-core)",
].join("\n");

/**
 * A temporary tree holding the cargo stub and the fixtures it answers from.
 *
 * @param {{core?: string, tauri?: string, owners?: Record<string, string>, failCore?: boolean, failInvert?: string}} fixtures
 *   `core` / `tauri` are the `cargo tree --prefix none` output for each
 *   Cargo world; `owners` maps a `--invert` target to its output.
 */
function makeTree(fixtures) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "check-linux-tls-"));
  const bin = path.join(root, "bin");
  const owners = path.join(root, "owners");
  fs.mkdirSync(bin, { recursive: true });
  fs.mkdirSync(owners, { recursive: true });
  fs.writeFileSync(path.join(root, "tree.core"), fixtures.core ?? CLEAN_TREE);
  fs.writeFileSync(path.join(root, "tree.tauri"), fixtures.tauri ?? CLEAN_TREE);
  if (fixtures.failCore) fs.writeFileSync(path.join(root, "fail-core"), "");
  if (fixtures.failInvert) {
    fs.writeFileSync(path.join(root, "fail-invert"), fixtures.failInvert);
  }
  for (const [target, text] of Object.entries(fixtures.owners ?? {})) {
    fs.writeFileSync(path.join(owners, target), text);
  }

  // Every call is recorded, so a test can prove which lookups the script made.
  fs.writeFileSync(
    path.join(bin, "cargo"),
    `#!/usr/bin/env bash
echo "cargo $*" >> "$TEST_CALLS_LOG"
manifest=""
inverted=""
while [ $# -gt 0 ]; do
  case "$1" in
    --manifest-path) manifest="$2"; shift ;;
    --invert) inverted="$2"; shift ;;
  esac
  shift
done
if [ -n "$inverted" ]; then
  if [ -f "$TEST_FIXTURE_ROOT/fail-invert" ] && [ "$inverted" = "$(cat "$TEST_FIXTURE_ROOT/fail-invert")" ]; then
    echo "cargo tree failed" >&2
    exit 7
  fi
  [ -f "$TEST_OWNERS_DIR/$inverted" ] && cat "$TEST_OWNERS_DIR/$inverted"
  exit 0
fi
case "$manifest" in
  Cargo.toml)
    if [ -f "$TEST_FIXTURE_ROOT/fail-core" ]; then
      echo "cargo tree failed" >&2
      exit 7
    fi
    cat "$TEST_FIXTURE_ROOT/tree.core" ;;
  *) cat "$TEST_FIXTURE_ROOT/tree.tauri" ;;
esac
`,
    { mode: 0o755 },
  );
  return {
    root,
    bin,
    calls: () => fs.readFileSync(path.join(root, "calls.log"), "utf8"),
  };
}

function run(interpreter, tree) {
  return spawnSync(interpreter, [script], {
    cwd: tree.root,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${tree.bin}:${process.env.PATH}`,
      TEST_FIXTURE_ROOT: tree.root,
      TEST_CALLS_LOG: path.join(tree.root, "calls.log"),
      TEST_OWNERS_DIR: path.join(tree.root, "owners"),
    },
  });
}

// `bash` as the shebang resolves it, plus the stock /bin/bash when that is a
// different (on macOS: 3.2) binary.
const SKIP =
  process.platform === "win32" ? { skip: "requires a POSIX shell" } : {};
const interpreters = ["bash"];
if (process.platform === "darwin" && fs.existsSync("/bin/bash")) {
  interpreters.push("/bin/bash");
}

for (const interpreter of interpreters) {
  const version =
    process.platform === "win32"
      ? "unavailable"
      : (spawnSync(interpreter, ["-c", 'echo "$BASH_VERSION"'], {
          encoding: "utf8",
        }).stdout?.trim() ?? "unavailable");

  test(
    `[${interpreter} ${version}] a tree without reqwest 0.13 passes`,
    SKIP,
    () => {
      // The regression: under bash 3.2 this run used to stop at `mapfile` with
      // exit 127 and never reach the verdict. With no 0.13 versions the array
      // is also empty, which `set -u` must tolerate.
      const tree = makeTree({});
      const result = run(interpreter, tree);
      assert.equal(result.status, 0, result.stderr);
      assert.match(
        result.stdout,
        /dependency policy passed for both Cargo worlds/,
      );
      assert.doesNotMatch(result.stderr, /mapfile/);
      assert.doesNotMatch(tree.calls(), /--invert reqwest@/);
    },
  );

  test(
    `[${interpreter} ${version}] every reqwest 0.13 version is checked for a Sentry owner`,
    SKIP,
    () => {
      const tree = makeTree({
        tauri: [
          "reqwest v0.13.2",
          "reqwest v0.13.2 (*)",
          "reqwest v0.13.5",
          "tauri v2.0.0",
        ].join("\n"),
        owners: {
          "reqwest@0.13.2": "reqwest v0.13.2\ntauri v2.0.0",
          "reqwest@0.13.5": "reqwest v0.13.5\ntauri-plugin-updater v2.0.0",
        },
      });
      const result = run(interpreter, tree);
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(tree.calls().trim().split("\n"), [
        "cargo tree --locked --manifest-path Cargo.toml --target x86_64-unknown-linux-gnu --prefix none",
        "cargo tree --locked --manifest-path crates/openhuman-app/Cargo.toml --target x86_64-unknown-linux-gnu --prefix none",
        "cargo tree --locked --manifest-path crates/openhuman-app/Cargo.toml --target x86_64-unknown-linux-gnu --prefix none --invert reqwest@0.13.2",
        "cargo tree --locked --manifest-path crates/openhuman-app/Cargo.toml --target x86_64-unknown-linux-gnu --prefix none --invert reqwest@0.13.5",
      ]);
    },
  );

  test(
    `[${interpreter} ${version}] a Sentry-owned reqwest 0.13 fails the policy`,
    SKIP,
    () => {
      // `--prefix none` makes the real inverted tree emit dependents at column 0.
      const tree = makeTree({
        tauri: "reqwest v0.13.2\nsentry v0.36.0",
        owners: { "reqwest@0.13.2": "reqwest v0.13.2\nsentry v0.36.0" },
      });
      const result = run(interpreter, tree);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Sentry owns reqwest 0\.13\.2 in tauri/);
      assert.match(tree.calls(), /--prefix none --invert reqwest@0\.13\.2/);
    },
  );

  test(
    `[${interpreter} ${version}] an aws-lc dependency fails the policy`,
    SKIP,
    () => {
      const tree = makeTree({ core: "openhuman v0.1.0\naws-lc-sys v0.21.0" });
      const result = run(interpreter, tree);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /aws-lc dependencies found in core/);
    },
  );

  test(
    `[${interpreter} ${version}] a failed dependency tree is a check error`,
    SKIP,
    () => {
      const tree = makeTree({ failCore: true });
      const result = run(interpreter, tree);
      assert.equal(result.status, 2);
      assert.match(result.stderr, /could not check core dependency tree/);
    },
  );

  test(
    `[${interpreter} ${version}] a real cargo tree failure is reported as a check error`,
    {
      ...SKIP,
      skip:
        SKIP.skip ||
        (spawnSync("cargo", ["--version"], { encoding: "utf8" }).status !== 0
          ? "requires Cargo"
          : false),
    },
    () => {
      // Run the checker outside a Cargo workspace so the real Cargo command
      // fails to load Cargo.toml. This exercises the production command/error
      // boundary without building dependencies or accessing the network.
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "check-linux-tls-real-cargo-"));
      const result = spawnSync(interpreter, [script], {
        cwd: root,
        encoding: "utf8",
        env: process.env,
      });
      assert.equal(result.status, 2);
      assert.match(result.stderr, /could not check core dependency tree/);
    },
  );

  test(
    `[${interpreter} ${version}] a failed reqwest owner query is a check error`,
    SKIP,
    () => {
      const tree = makeTree({
        tauri: "reqwest v0.13.2",
        failInvert: "reqwest@0.13.2",
      });
      const result = run(interpreter, tree);
      assert.equal(result.status, 2);
      assert.match(
        result.stderr,
        /could not check reqwest 0\.13\.2 owners in tauri/,
      );
    },
  );

  test(
    `[${interpreter} ${version}] a failed TLS owner query is a check error`,
    SKIP,
    () => {
      const tree = makeTree({
        tauri: "native-tls v0.2.14",
        failInvert: "native-tls",
      });
      const result = run(interpreter, tree);
      assert.equal(result.status, 2);
      assert.match(result.stderr, /could not check native-tls owners in tauri/);
    },
  );

  test(
    `[${interpreter} ${version}] a Sentry-owned TLS package fails the policy`,
    SKIP,
    () => {
      const tree = makeTree({
        tauri: "native-tls v0.2.14",
        owners: { "native-tls": SENTRY_OWNER_TREE },
      });
      assert.match(SENTRY_OWNER_TREE, /^sentry v0\.47\.0$/m);
      const result = run(interpreter, tree);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Sentry owns native-tls in tauri/);
      assert.deepEqual(tree.calls().trim().split("\n"), [
        "cargo tree --locked --manifest-path Cargo.toml --target x86_64-unknown-linux-gnu --prefix none",
        "cargo tree --locked --manifest-path crates/openhuman-app/Cargo.toml --target x86_64-unknown-linux-gnu --prefix none",
        "cargo tree --locked --manifest-path crates/openhuman-app/Cargo.toml --target x86_64-unknown-linux-gnu --prefix none --invert native-tls",
      ]);
    },
  );

  test(
    `[${interpreter} ${version}] an early TLS match in a large tree still checks owners`,
    SKIP,
    () => {
      const tree = makeTree({
        tauri: `native-tls v0.2.14\n${"other-package v1.0.0\n".repeat(20000)}`,
        owners: { "native-tls": SENTRY_OWNER_TREE },
      });
      const result = run(interpreter, tree);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Sentry owns native-tls in tauri/);
      assert.match(tree.calls(), /--invert native-tls/);
    },
  );
}
