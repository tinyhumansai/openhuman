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

/**
 * A temporary tree holding the cargo stub and the fixtures it answers from.
 *
 * @param {{core?: string, tauri?: string, owners?: Record<string, string>}} fixtures
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
  for (const [target, text] of Object.entries(fixtures.owners ?? {})) {
    fs.writeFileSync(path.join(owners, target), text);
  }

  // Every call is recorded, so a test can prove which lookups the script made.
  fs.writeFileSync(
    path.join(bin, "cargo"),
    `#!/usr/bin/env bash
echo "cargo $*" >> "${path.join(root, "calls.log")}"
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
  [ -f "${owners}/$inverted" ] && cat "${owners}/$inverted"
  exit 0
fi
case "$manifest" in
  Cargo.toml) cat "${root}/tree.core" ;;
  *) cat "${root}/tree.tauri" ;;
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
    env: { ...process.env, PATH: `${tree.bin}:${process.env.PATH}` },
  });
}

// `bash` as the shebang resolves it, plus the stock /bin/bash when that is a
// different (on macOS: 3.2) binary.
const interpreters = ["bash"];
if (process.platform === "darwin" && fs.existsSync("/bin/bash")) {
  interpreters.push("/bin/bash");
}

for (const interpreter of interpreters) {
  const version = spawnSync(interpreter, ["-c", 'echo "$BASH_VERSION"'], {
    encoding: "utf8",
  }).stdout.trim();

  test(`[${interpreter} ${version}] a tree without reqwest 0.13 passes`, () => {
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
  });

  test(`[${interpreter} ${version}] every reqwest 0.13 version is checked for a Sentry owner`, () => {
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
    const inverted = [...tree.calls().matchAll(/--invert (reqwest@\S+)/g)].map(
      (m) => m[1],
    );
    assert.deepEqual(inverted, ["reqwest@0.13.2", "reqwest@0.13.5"]);
  });

  test(`[${interpreter} ${version}] a Sentry-owned reqwest 0.13 fails the policy`, () => {
    // The owners fixture is in the shape the script greps for (`^sentry v`),
    // not the indented shape `cargo tree --invert` really prints; that
    // mismatch is #6602 and is out of scope here.
    const tree = makeTree({
      tauri: "reqwest v0.13.2\nsentry v0.36.0",
      owners: { "reqwest@0.13.2": "reqwest v0.13.2\nsentry v0.36.0" },
    });
    const result = run(interpreter, tree);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Sentry owns reqwest 0\.13\.2 in tauri/);
  });

  test(`[${interpreter} ${version}] an aws-lc dependency fails the policy`, () => {
    const tree = makeTree({ core: "openhuman v0.1.0\naws-lc-sys v0.21.0" });
    const result = run(interpreter, tree);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /aws-lc dependencies found in core/);
  });
}
