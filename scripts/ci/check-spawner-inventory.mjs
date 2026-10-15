#!/usr/bin/env node
// Process-spawner inventory (issue #7328, phase 5).
//
// Every place the host starts an OS process is a place untrusted input can
// reach a shell. This check keeps a committed inventory of those sites
// (scripts/ci/spawner-inventory.json) in step with the code: a new spawn site
// fails CI until a row names its source trust, plan, executor and test, and a
// removed one must leave the inventory.
//
// Each row:
//   file     repo-relative path of a non-test Rust file
//   spawns   number of `Command::new(` sites plus `shell_exec::run_host(` calls
//   trust    "operator" (host-authored argv), "untrusted" (model, flow, skill,
//            MCP or user-supplied argv), or "host-probe" (fixed diagnostics)
//   plan     for untrusted rows, the sandbox plan family that governs it
//   executor how it is launched: "direct" (no jail yet), "jailed" (through
//            tinybox), or "n/a" for operator and host-probe rows
//   test     test that pins the row, or "pending" while the row is "direct"
//
// Untrusted rows with executor "direct" are the migration backlog; they are
// allowed only while they carry `test: "pending"` and are counted in the
// summary. A new unclassified row (trust "unclassified") always fails.
//
//   node scripts/ci/check-spawner-inventory.mjs
//   node scripts/ci/check-spawner-inventory.mjs --write   (add/refresh rows)
//   node scripts/ci/check-spawner-inventory.mjs --root <dir>   (tests)

import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const rootFlag = process.argv.indexOf("--root");
const repoRoot =
  rootFlag === -1
    ? resolve(dirname(fileURLToPath(import.meta.url)), "../..")
    : resolve(process.argv[rootFlag + 1]);
const inventoryPath = resolve(repoRoot, "scripts/ci/spawner-inventory.json");
const write = process.argv.includes("--write");

const SPAWN = /\b[A-Za-z]*Command::new\s*\(/g;
// The host shell helper starts a process on the caller's behalf, so its call
// sites are spawn sites of the caller.
const HELPER = /\bshell_exec::run_host\s*\(/g;
const PROCESS_CMD = /process::(?:Command\b|\{[^}]*\bCommand\b)/;
const TRUSTS = new Set(["operator", "untrusted", "host-probe", "unclassified"]);

async function* walk(dir) {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    if (e.name === "target" || e.name === "node_modules") continue;
    const p = resolve(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (e.name.endsWith(".rs")) yield p;
  }
}

function isTestFile(rel) {
  return (
    /_tests\.rs$/.test(rel) ||
    /_fixture\.rs$/.test(rel) ||
    rel.includes("/tests/") ||
    rel.endsWith("module_test_process.rs")
  );
}

/** Spawn-site counts per non-test file under crates/<crate>/src. */
export async function scan(root) {
  const found = new Map();
  const crates = resolve(root, "crates");
  let names = [];
  try {
    names = await readdir(crates);
  } catch {
    return found;
  }
  for (const c of names) {
    const src = resolve(crates, c, "src");
    try {
      for await (const file of walk(src)) {
        const rel = relative(root, file).split(sep).join("/");
        if (isTestFile(rel)) continue;
        const text = await readFile(file, "utf8");
        const direct = PROCESS_CMD.test(text) ? (text.match(SPAWN) ?? []).length : 0;
        const n = direct + (text.match(HELPER) ?? []).length;
        if (n > 0) found.set(rel, n);
      }
    } catch {
      /* crate without src/ */
    }
  }
  return found;
}

/** Problems between the scanned sites and the committed rows. */
export function diff(found, rows) {
  const problems = [];
  const byFile = new Map(rows.map((r) => [r.file, r]));
  for (const [file, n] of found) {
    const row = byFile.get(file);
    if (!row) {
      problems.push(`${file}: ${n} spawn site(s) not in the inventory`);
      continue;
    }
    if (row.spawns !== n)
      problems.push(`${file}: inventory says ${row.spawns} spawn(s), code has ${n}`);
  }
  for (const row of rows) {
    if (!found.has(row.file)) problems.push(`${row.file}: stale inventory row (no spawn site)`);
    if (!TRUSTS.has(row.trust)) problems.push(`${row.file}: unknown trust "${row.trust}"`);
    if (row.trust === "unclassified")
      problems.push(`${row.file}: classify trust, plan, executor and test`);
    if (row.trust === "untrusted") {
      if (!row.plan) problems.push(`${row.file}: untrusted row names no plan`);
      if (!["direct", "jailed"].includes(row.executor))
        problems.push(`${row.file}: untrusted row needs executor direct|jailed`);
      if (row.executor === "jailed" && (!row.test || row.test === "pending"))
        problems.push(`${row.file}: jailed row needs a real test`);
      if (row.executor === "direct" && row.test !== "pending")
        problems.push(`${row.file}: direct untrusted row must carry test "pending"`);
    }
  }
  return problems;
}

async function main() {
  const found = await scan(repoRoot);
  let rows = [];
  try {
    rows = JSON.parse(await readFile(inventoryPath, "utf8")).rows;
  } catch {
    /* first write */
  }
  if (write) {
    const byFile = new Map(rows.map((r) => [r.file, r]));
    rows = [...found.keys()].sort().map((file) => ({
      ...(byFile.get(file) ?? {
        trust: "unclassified",
        plan: "",
        executor: "n/a",
        test: "",
      }),
      file,
      spawns: found.get(file),
    }));
    // Keep `file` first for readable diffs.
    rows = rows.map(({ file, spawns, ...rest }) => ({ file, spawns, ...rest }));
    await writeFile(inventoryPath, JSON.stringify({ rows }, null, 2) + "\n");
    console.log(`spawner inventory: wrote ${rows.length} rows`);
    return;
  }
  const problems = diff(found, rows);
  const backlog = rows.filter((r) => r.trust === "untrusted" && r.executor === "direct");
  if (problems.length) {
    console.error("spawner inventory out of date:");
    for (const p of problems) console.error(`  ${p}`);
    console.error("run: node scripts/ci/check-spawner-inventory.mjs --write, then classify new rows");
    process.exit(1);
  }
  console.log(
    `spawner inventory: ${rows.length} rows ok; ${backlog.length} untrusted spawner(s) still direct`,
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
