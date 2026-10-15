// Unit tests for scripts/ci/check-spawner-inventory.mjs.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { diff } from "../ci/check-spawner-inventory.mjs";

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const script = path.join(repoRoot, "scripts", "ci", "check-spawner-inventory.mjs");
const SPAWNER = `use std::process::Command;\nfn f(){ let _ = Command::new("sh"); }\n`;

function run(files, rows, args = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-spawner-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  if (rows) {
    const abs = path.join(root, "scripts", "ci", "spawner-inventory.json");
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, JSON.stringify({ rows }));
  }
  const r = spawnSync(process.execPath, [script, "--root", root, ...args], { encoding: "utf8" });
  fs.rmSync(root, { recursive: true, force: true });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}

const operator = (file, spawns = 1) => ({
  file,
  spawns,
  trust: "operator",
  plan: "n/a",
  executor: "n/a",
  test: "n/a",
});

test("a new spawn site not in the inventory fails", () => {
  const r = run({ "crates/a/src/lib.rs": SPAWNER }, []);
  assert.equal(r.status, 1);
  assert.match(r.out, /not in the inventory/);
});

test("a listed site passes and a stale row fails", () => {
  assert.equal(
    run({ "crates/a/src/lib.rs": SPAWNER }, [operator("crates/a/src/lib.rs")]).status,
    0,
  );
  const stale = run({ "crates/a/src/lib.rs": "fn f(){}\n" }, [operator("crates/a/src/lib.rs")]);
  assert.equal(stale.status, 1);
  assert.match(stale.out, /stale inventory row/);
});

test("test files are not spawn sites", () => {
  const r = run({ "crates/a/src/lib_tests.rs": SPAWNER }, []);
  assert.equal(r.status, 0);
});

test("clap Command::new is not a process spawn", () => {
  const r = run({ "crates/a/src/lib.rs": `use clap::Command;\nfn f(){ Command::new("x"); }\n` }, []);
  assert.equal(r.status, 0);
});

test("shell_exec::run_host call sites count as spawns", () => {
  const r = run({ "crates/a/src/lib.rs": "fn f(){ shell_exec::run_host(a); }\n" }, []);
  assert.equal(r.status, 1);
});

test("untrusted rows need a plan and a pending test while direct", () => {
  const bad = diff(new Map([["f.rs", 1]]), [
    { file: "f.rs", spawns: 1, trust: "untrusted", plan: "", executor: "direct", test: "x" },
  ]);
  assert.equal(bad.length, 2);
  const jailedNoTest = diff(new Map([["f.rs", 1]]), [
    { file: "f.rs", spawns: 1, trust: "untrusted", plan: "p", executor: "jailed", test: "pending" },
  ]);
  assert.match(jailedNoTest[0], /real test/);
});

test("unclassified rows always fail and --write adds them", () => {
  const w = run({ "crates/a/src/lib.rs": SPAWNER }, [], ["--write"]);
  assert.equal(w.status, 0);
  assert.match(
    diff(new Map([["f.rs", 1]]), [{ file: "f.rs", spawns: 1, trust: "unclassified" }]).join(),
    /classify/,
  );
});

test("the committed inventory matches the repository", () => {
  const r = spawnSync(process.execPath, [script], { encoding: "utf8", cwd: repoRoot });
  assert.equal(r.status, 0, `${r.stdout}${r.stderr}`);
});
