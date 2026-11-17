// Unit tests for scripts/ci/check-ignored-tests.mjs — the per-crate `#[ignore]`
// ratchet that runs in the `rust-quality` lane.
//
// Each test pins one clause the gate would be useless without. Delete that
// clause and exactly one test here goes red. Two of them exist because the
// naive version of this gate got them wrong:
//
//   - `counts attributes, not prose` — an unanchored `#\[ignore` match also hits
//     doc comments that merely DISCUSS ignoring (nine such lines in the real
//     tree), so deleting a comment would have earned ratchet credit.
//   - `counts cfg_attr(..., ignore)` — a conditional ignore is one attribute,
//     counted like a plain one, so scoping an ignore to a platform cannot
//     shrink the number while the test stays skipped.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const script = path.join(repoRoot, "scripts", "ci", "check-ignored-tests.mjs");

/**
 * Build a throwaway tree, then run the gate over it.
 *
 * @param {Record<string,string>} files repo-relative path -> contents
 * @param {Record<string,number>|null} baseline written to the baseline path, or
 *   omitted entirely to exercise the missing-baseline path
 * @param {string[]} args extra argv
 */
function run(files, baseline, args = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-ignore-ratchet-"));
  // The gate lists `crates/` to discover buckets, so it must always exist.
  fs.mkdirSync(path.join(root, "crates"), { recursive: true });
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  if (baseline) {
    const abs = path.join(root, "scripts", "ci", "ignored-test-baseline.json");
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, `${JSON.stringify(baseline, null, 2)}\n`);
  }
  const result = spawnSync(process.execPath, [script, "--root", root, ...args], {
    encoding: "utf8",
  });
  fs.rmSync(root, { recursive: true, force: true });
  return { status: result.status, out: `${result.stdout}${result.stderr}` };
}

const IGNORED_TEST = '#[test]\n#[ignore = "needs a daemon"]\nfn skipped() {}\n';

test("passes when every bucket matches its baseline", () => {
  const { status, out } = run(
    { "crates/demo/src/lib.rs": IGNORED_TEST },
    { demo: 1 },
  );
  assert.equal(status, 0, out);
  assert.match(out, /ratchet holds \(1 /);
});

test("fails when a crate gains an ignored test", () => {
  const { status, out } = run(
    { "crates/demo/src/lib.rs": IGNORED_TEST + IGNORED_TEST },
    { demo: 1 },
  );
  assert.equal(status, 1);
  assert.match(out, /demo: 2 '#\[ignore\]' attributes, baseline 1 \(\+1\)/);
  assert.match(out, /only goes down/);
});

test("fails when a crate loses one, demanding the baseline be tightened", () => {
  const { status, out } = run({ "crates/demo/src/lib.rs": IGNORED_TEST }, { demo: 2 });
  assert.equal(status, 1);
  assert.match(out, /demo: 1 '#\[ignore\]' attributes, baseline 2 \(-1\)/);
  assert.match(out, /Tighten the baseline/);
});

test("a crate absent from the baseline is held at zero, not grandfathered", () => {
  const { status, out } = run({ "crates/fresh/src/lib.rs": IGNORED_TEST }, {});
  assert.equal(status, 1);
  assert.match(out, /fresh: 1 '#\[ignore\]' attributes, baseline 0 \(\+1\)/);
});

test("counts attributes, not prose that mentions them", () => {
  const { status, out } = run(
    {
      "crates/demo/src/lib.rs":
        "// We deliberately do not `#[ignore]` this one.\n" +
        "/// See the argument about #[ignore] in the module docs.\n" +
        "//! Nothing here is #[ignore]d.\n" +
        IGNORED_TEST,
    },
    { demo: 1 },
  );
  assert.equal(status, 0, out);
});

test("counts cfg_attr(..., ignore) like a plain ignore", () => {
  const files = { "crates/demo/src/lib.rs": "#[test]\n#[cfg_attr(unix, ignore)]\nfn skipped() {}\n" };
  assert.equal(run(files, { demo: 1 }).status, 0);
  const { status, out } = run(files, { demo: 0 });
  assert.equal(status, 1);
  assert.match(out, /demo: 1 '#\[ignore\]' attributes, baseline 0/);
});

test("the root tests/ tree is its own bucket, not folded into a crate", () => {
  const { status, out } = run(
    { "tests/thing_e2e.rs": IGNORED_TEST, "crates/demo/src/lib.rs": "pub fn ok() {}\n" },
    { demo: 0, tests: 0 },
  );
  assert.equal(status, 1);
  assert.match(out, /^ {2}tests: 1 /m);
  assert.doesNotMatch(out, /demo: /);
});

test("a baselined bucket that vanished must be removed from the baseline", () => {
  const { status, out } = run({ "crates/demo/src/lib.rs": IGNORED_TEST }, { demo: 1, gone: 3 });
  assert.equal(status, 1);
  assert.match(out, /gone: baselined but no longer scanned/);
});

test("skips target/ and node_modules/, which hold generated copies", () => {
  const { status, out } = run(
    {
      "crates/demo/src/lib.rs": IGNORED_TEST,
      "crates/demo/target/debug/build/generated.rs": IGNORED_TEST + IGNORED_TEST,
      "crates/demo/node_modules/pkg/vendored.rs": IGNORED_TEST,
    },
    { demo: 1 },
  );
  assert.equal(status, 0, out);
});

test("--write-baseline records what it scanned", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-ignore-ratchet-w-"));
  fs.mkdirSync(path.join(root, "crates", "demo", "src"), { recursive: true });
  fs.writeFileSync(path.join(root, "crates", "demo", "src", "lib.rs"), IGNORED_TEST);
  fs.mkdirSync(path.join(root, "scripts", "ci"), { recursive: true });
  const written = spawnSync(process.execPath, [script, "--root", root, "--write-baseline"], {
    encoding: "utf8",
  });
  assert.equal(written.status, 0, `${written.stdout}${written.stderr}`);
  const baseline = JSON.parse(
    fs.readFileSync(path.join(root, "scripts", "ci", "ignored-test-baseline.json"), "utf8"),
  );
  assert.deepEqual(baseline, { demo: 1, tests: 0 });
  fs.rmSync(root, { recursive: true, force: true });
});
