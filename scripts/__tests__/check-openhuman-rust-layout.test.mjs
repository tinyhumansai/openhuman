// Unit tests for scripts/ci/check-openhuman-rust-layout.mjs.
//
// The gate reads a fixed tree relative to the cwd and hard-codes its legacy
// pins, so each test copies the script into a throwaway tree, swaps its pin
// list for the test's own, and runs it there. Each clause of the line limit —
// scope, warn band, ratchet, duplicate pins — has a test that goes red without
// it.

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
const scriptRel = path.join("scripts", "ci", "check-openhuman-rust-layout.mjs");
const scriptSource = fs.readFileSync(path.join(repoRoot, scriptRel), "utf8");
const SRC = "crates/openhuman-core/src";
const PINS_RE = /const LEGACY_LIMIT_ENTRIES = \[[\s\S]*?\n\];/;

/** A Rust file of exactly `n` lines, as the gate counts them. */
const lines = (n) => "//\n".repeat(n);

/**
 * Run the gate over a throwaway tree.
 *
 * @param {Record<string,number>} files path under `SRC` -> line count
 * @param {Array<[string, number]>} pins legacy pins, paths under `SRC`
 */
function run(files, pins = []) {
  const entries = pins.map(([file, limit]) => [`${SRC}/${file}`, limit]);
  assert.match(
    scriptSource,
    PINS_RE,
    "fixture: the gate's pin list was not found",
  );
  const source = scriptSource.replace(
    PINS_RE,
    `const LEGACY_LIMIT_ENTRIES = ${JSON.stringify(entries)};`,
  );
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-rust-layout-"));
  for (const dir of [
    SRC,
    "crates/openhuman-cli",
    "tests",
    "examples",
    "scripts/ci",
    "scripts/lib",
  ])
    fs.mkdirSync(path.join(root, dir), { recursive: true });
  fs.writeFileSync(path.join(root, "crates/openhuman-core/Cargo.toml"), "");
  fs.writeFileSync(path.join(root, "crates/openhuman-cli/Cargo.toml"), "");
  for (const [file, n] of Object.entries(files)) {
    const abs = path.join(root, SRC, file);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, lines(n));
  }
  fs.writeFileSync(path.join(root, scriptRel), source);
  fs.copyFileSync(
    path.join(repoRoot, "scripts", "lib", "root-rust-targets.mjs"),
    path.join(root, "scripts", "lib", "root-rust-targets.mjs"),
  );
  const result = spawnSync(process.execPath, [scriptRel], {
    cwd: root,
    encoding: "utf8",
  });
  fs.rmSync(root, { recursive: true, force: true });
  return { status: result.status, out: `${result.stdout}${result.stderr}` };
}

test("passes at exactly the limit and at exactly a pin", () => {
  const { status, out } = run({ "a.rs": 750, "big.rs": 900 }, [
    ["big.rs", 900],
  ]);
  assert.equal(status, 0, out);
});

test("fails one line over the limit", () => {
  const { status, out } = run({ "a.rs": 751 });
  assert.equal(status, 1);
  assert.match(out, /a\.rs: 751 lines \(limit 750\)/);
});

test("rejects test.rs, tests.rs and the singular <name>_test.rs", () => {
  for (const name of ["test.rs", "tests.rs", "thing_test.rs"]) {
    const { status, out } = run({ [name]: 1 });
    assert.equal(status, 1, name);
    assert.match(out, /must use a descriptive \*_tests\.rs filename/);
  }
  const ok = run({ "thing_tests.rs": 1 });
  assert.equal(ok.status, 0, ok.out);
});

test("warns, without failing, on a file inside the warn band", () => {
  const { status, out } = run({ "near.rs": 740, "far.rs": 725 });
  assert.equal(status, 0, out);
  assert.match(out, /near\.rs: 740 lines, 10 below the 750-line limit/);
  assert.doesNotMatch(out, /far\.rs/);
});

test("enforces the limit under core/, which used to be pruned by name", () => {
  const { status, out } = run({ "core/observability.rs": 800, "lib.rs": 800 });
  assert.equal(status, 1);
  assert.match(out, /core\/observability\.rs: 800 lines \(limit 750\)/);
  assert.match(out, /lib\.rs: 800 lines \(limit 750\)/);
});

test("a pinned file that shrank must lower its pin", () => {
  const { status, out } = run({ "big.rs": 850 }, [["big.rs", 900]]);
  assert.equal(status, 1);
  assert.match(
    out,
    /big\.rs: 850 lines, below its legacy pin 900; lower the pin to 850/,
  );
});

test("a pinned file that fits the limit must drop its pin", () => {
  const { status, out } = run({ "big.rs": 700 }, [["big.rs", 900]]);
  assert.equal(status, 1);
  assert.match(
    out,
    /big\.rs: 700 lines now fits the 750-line limit; remove its legacy exception/,
  );
});

test("a file pinned twice fails, because a Map keeps only the last pin", () => {
  const { status, out } = run({ "big.rs": 900 }, [
    ["big.rs", 800],
    ["big.rs", 900],
  ]);
  assert.equal(status, 1);
  assert.match(out, /big\.rs: duplicate legacy exception/);
});

test("the real pin list has no duplicate entry", () => {
  const files = [
    ...scriptSource.match(PINS_RE)[0].matchAll(/"(crates\/[^"]+\.rs)"/g),
  ].map((m) => m[1]);
  assert.ok(files.length > 0, "fixture: no pins parsed");
  assert.equal(new Set(files).size, files.length);
});
