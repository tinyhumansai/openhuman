// Unit tests for scripts/ci/check-storage-bypass.mjs, the ratchet over state
// that is persisted behind the storage port.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  ALLOW,
  allowed,
  compare,
  scan,
  withoutAllowed,
} from "../ci/check-storage-bypass.mjs";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const script = path.join(repoRoot, "scripts", "ci", "check-storage-bypass.mjs");
const SRC = "crates/openhuman-core/src";

function run(files, baseline, args = []) {
  const root = fs.mkdtempSync(
    path.join(os.tmpdir(), "openhuman-storage-bypass-"),
  );
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  const written = path.join(
    root,
    "scripts",
    "ci",
    "storage-bypass-baseline.json",
  );
  if (baseline) {
    fs.mkdirSync(path.dirname(written), { recursive: true });
    fs.writeFileSync(written, `${JSON.stringify(baseline, null, 2)}\n`);
  }
  const result = spawnSync(
    process.execPath,
    [script, "--root", root, ...args],
    {
      encoding: "utf8",
    },
  );
  const after = fs.existsSync(written)
    ? JSON.parse(fs.readFileSync(written, "utf8"))
    : null;
  fs.rmSync(root, { recursive: true, force: true });
  return {
    status: result.status,
    out: `${result.stdout}${result.stderr}`,
    after,
  };
}

test("flags a database open and a raw file write", () => {
  const found = scan(
    "x.rs",
    [
      "let c = Connection::open(&path)?;",
      "let c = rusqlite::Connection::open(path)?;",
      "std::fs::write(&p, body)?;",
      "let f = File::create(p)?;",
      "let f = OpenOptions::new().append(true).open(p)?;",
    ].join("\n"),
  );
  assert.deepEqual(
    found.map((f) => f.rule),
    ["sqlite-open", "sqlite-open", "json-write", "json-write", "json-write"],
  );
});

test("ignores comments and doc lines", () => {
  const found = scan(
    "x.rs",
    [
      "// Connection::open(&p)",
      "//! fs::write(p, b)",
      "let s = 1; // File::create(p)",
    ].join("\n"),
  );
  assert.deepEqual(found, []);
});

test("compare reports added and stale sites", () => {
  const finding = {
    rule: "sqlite-open",
    path: "a.rs",
    line: 3,
    text: "x",
    occurrence: 1,
  };
  const other = {
    rule: "json-write",
    path: "b.rs",
    line: 4,
    text: "y",
    occurrence: 1,
  };
  const { added, stale } = compare([finding], [other]);
  assert.deepEqual(added, [finding]);
  assert.deepEqual(stale, [other]);
});

test("every allowlisted path carries a reason and known rules", () => {
  for (const [file, entries] of ALLOW) {
    assert.ok(file.startsWith(`${SRC}/`), file);
    for (const entry of entries) {
      assert.ok(entry.reason.length > 20, file);
      assert.ok(["sqlite-open", "json-write"].includes(entry.rule));
      assert.ok(entry.site.length > 5, file);
    }
  }
  const state = `${SRC}/config/workspace/state.rs`;
  const [site] = scan(state, "let conn = Connection::open(db_path)?;");
  assert.ok(allowed(state, site));
  assert.ok(!allowed(state, { ...site, rule: "json-write" }));
  assert.ok(
    !allowed(state, { ...site, text: "let c = Connection::open(other)?;" }),
  );
});

test("a new database open fails the check", () => {
  const { status, out } = run(
    { [`${SRC}/foo/store.rs`]: "fn f() { let c = Connection::open(p); }\n" },
    [],
  );
  assert.equal(status, 1);
  assert.match(out, /New storage-bypass sites/);
  assert.match(
    out,
    /sqlite-open: crates\/openhuman-core\/src\/foo\/store\.rs:1/,
  );
});

test("the storage module, tests and allowlisted sites are not flagged", () => {
  const { status } = run(
    {
      [`${SRC}/storage/driver.rs`]: "fn f() { let c = Connection::open(p); }\n",
      [`${SRC}/foo/store_tests.rs`]: "fn f() { std::fs::write(p, b); }\n",
      [`${SRC}/foo/tests/common.rs`]: "fn f() { File::create(p); }\n",
      [`${SRC}/security/approval/store_import.rs`]:
        "fn f() {\n    let conn = Connection::open(&db_path).with_context(|| {\n}\n",
    },
    [],
  );
  assert.equal(status, 0);
});

test("a fixed site left in the baseline fails until removed", () => {
  const { status, out } = run({ [`${SRC}/foo/store.rs`]: "fn f() {}\n" }, [
    {
      rule: "sqlite-open",
      path: `${SRC}/foo/store.rs`,
      line: 1,
      text: "x",
      occurrence: 1,
    },
  ]);
  assert.equal(status, 1);
  assert.match(out, /Fixed sites still in the baseline/);
});

test("--write-baseline records the current sites", () => {
  const { status, after } = run(
    { [`${SRC}/foo/store.rs`]: "fn f() { std::fs::write(p, b); }\n" },
    null,
    ["--write-baseline"],
  );
  assert.equal(status, 0);
  assert.equal(after.length, 1);
  assert.equal(after[0].rule, "json-write");
});

test("an allowlisted file may hold only its pinned site", () => {
  const file = `${SRC}/desktop/control/ops.rs`;
  const pinned = "let mut temporary = tempfile::NamedTempFile::new_in(parent)";
  const found = scan(
    file,
    `${pinned}\nstd::fs::write(c, d);\nConnection::open(p);\n`,
  );
  const kept = withoutAllowed(file, found);
  assert.deepEqual(
    kept.map((f) => [f.rule, f.line]),
    [
      ["json-write", 2],
      ["sqlite-open", 3],
    ],
  );
  // The pinned line moving elsewhere in the file is still allowed; a new write is not.
  assert.equal(withoutAllowed(file, scan(file, `x();\n${pinned}\n`)).length, 0);
});

test("a call split across lines is still found", () => {
  const found = scan(
    "x.rs",
    "let c = Connection::open\n    (&path)?;\nstd::fs::write\n(p, b)?;\n",
  );
  assert.deepEqual(
    found.map((f) => [f.rule, f.line]),
    [
      ["sqlite-open", 1],
      ["json-write", 3],
    ],
  );
});

test("a verbatim copy of an allowlisted line is still a finding", () => {
  const file = `${SRC}/security/approval/store_import.rs`;
  const line = "let conn = Connection::open(&db_path).with_context(|| {";
  const kept = withoutAllowed(file, scan(file, `${line}\n});\n${line}\n});\n`));
  assert.deepEqual(
    kept.map((f) => [f.line, f.occurrence]),
    [[3, 2]],
  );
});
