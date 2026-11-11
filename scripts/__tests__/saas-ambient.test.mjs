// Unit tests for scripts/ci/check-saas-ambient.mjs — the ratchet over code
// that reaches past a SaaS user agent's CoreContext.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { codeOf, compare, exempt, scan } from "../ci/check-saas-ambient.mjs";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const script = path.join(repoRoot, "scripts", "ci", "check-saas-ambient.mjs");
const SRC = "crates/openhuman-core/src";

function run(files, baseline, args = []) {
  const root = fs.mkdtempSync(
    path.join(os.tmpdir(), "openhuman-saas-ambient-"),
  );
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  if (baseline) {
    const abs = path.join(root, "scripts", "ci", "saas-ambient-baseline.json");
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, `${JSON.stringify(baseline, null, 2)}\n`);
  }
  const result = spawnSync(
    process.execPath,
    [script, "--root", root, ...args],
    {
      encoding: "utf8",
    },
  );
  const written = path.join(
    root,
    "scripts",
    "ci",
    "saas-ambient-baseline.json",
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

test("flags every rule", () => {
  const found = scan(
    "x.rs",
    [
      "tokio::spawn(async {});",
      "let h = tokio::task::spawn_blocking(|| 1);",
      "std::thread::spawn(|| {});",
      "let c = Config::load_or_init().await?;",
      'std::env::set_var("A", "b");',
      "let h = dirs::home_dir();",
    ].join("\n"),
  );
  assert.deepEqual(
    found.map((f) => f.rule),
    [
      "bare-spawn",
      "bare-spawn",
      "bare-spawn",
      "ambient-config",
      "env-mutation",
      "home-dir",
    ],
  );
});

test("ignores comments, docs and the scoped helpers", () => {
  const found = scan(
    "x.rs",
    [
      "// tokio::spawn(fut) drops the scope",
      "/// call Config::load_or_init() only in single-user code",
      "spawn_scoped(async {});",
      'let s = "// not a comment"; tokio::spawn(f);',
    ].join("\n"),
  );
  assert.deepEqual(
    found.map((f) => f.rule),
    ["bare-spawn"],
  );
  assert.equal(codeOf("a(); // tokio::spawn("), "a(); ");
});

test("counts repeated identical lines as separate occurrences", () => {
  const found = scan("x.rs", "tokio::spawn(f);\ntokio::spawn(f);\n");
  assert.deepEqual(
    found.map((f) => f.occurrence),
    [1, 2],
  );
});

test("a moved line is not a new site, a new line is", () => {
  const before = scan("x.rs", "tokio::spawn(f);\n");
  const moved = scan("x.rs", "\n\ntokio::spawn(f);\n");
  assert.deepEqual(compare(moved, before), { added: [], stale: [] });
  const grown = scan("x.rs", "tokio::spawn(f);\ntokio::spawn(g);\n");
  assert.equal(compare(grown, before).added.length, 1);
});

test("passes on a matching baseline and skips test files and test support helpers", () => {
  const files = {
    [`${SRC}/a.rs`]: "tokio::spawn(f);\n",
    [`${SRC}/a_tests.rs`]: "tokio::spawn(f);\n",
    [`${SRC}/inference/provider/model_listing_test_support.rs`]:
      "tokio::spawn(f);\n",
    [`${SRC}/inference/provider/catalog_listing.rs`]:
      "Config::load_or_init();\n",
    [`${SRC}/core/runtime/spawn.rs`]: "tokio::spawn(f);\n",
  };
  const first = run(files, null, ["--write-baseline"]);
  assert.equal(first.status, 0, first.out);
  assert.equal(
    first.after.length,
    2,
    "only ordinary production sites are scanned",
  );
  assert.deepEqual(
    first.after.map(({ path, rule }) => ({ path, rule })),
    [
      { path: `${SRC}/a.rs`, rule: "bare-spawn" },
      {
        path: `${SRC}/inference/provider/catalog_listing.rs`,
        rule: "ambient-config",
      },
    ],
  );
  const unbaselined = run(files, []);
  assert.equal(
    unbaselined.status,
    1,
    "production ambient config reads still fail",
  );
  assert.match(
    unbaselined.out,
    /ambient-config: crates\/openhuman-core\/src\/inference\/provider\/catalog_listing\.rs:1/,
  );
  assert.doesNotMatch(unbaselined.out, /model_listing_test_support\.rs/);
  const again = run(files, first.after);
  assert.equal(again.status, 0, again.out);
  assert.match(again.out, /ratchet holds \(2 /);
});

test("fails on a new site and names the fix", () => {
  const { status, out } = run(
    { [`${SRC}/a.rs`]: "Config::load_or_init();\n" },
    [],
  );
  assert.equal(status, 1);
  assert.match(out, /ambient-config: crates\/openhuman-core\/src\/a.rs:1/);
  assert.match(out, /load_config_with_timeout/);
});

test("fails on a fixed site still in the baseline", () => {
  const baseline = scan(`${SRC}/a.rs`, "tokio::spawn(f);\n");
  const { status, out } = run(
    { [`${SRC}/a.rs`]: "spawn_scoped(f);\n" },
    baseline,
  );
  assert.equal(status, 1);
  assert.match(out, /remove them/);
});

test("flags ambient context reads in a tenant-keyed path", () => {
  const found = scan(
    `${SRC}/web_chat/ops/state.rs`,
    [
      "let agent = crate::core::runtime::CoreContext::current()",
      "    .and_then(|ctx| ctx.session_agent().map(str::to_owned));",
      "let overlay = overlay.session_agent(id);",
      "let tenant = crate::core::runtime::current_tenant()?;",
    ].join("\n"),
  );
  assert.deepEqual(
    found.map((f) => [f.rule, f.line]),
    [
      ["ambient-context", 1],
      ["ambient-context", 2],
    ],
  );
});

test("the runtime itself may read the ambient context", () => {
  const finding = { rule: "ambient-context" };
  assert.equal(exempt(`${SRC}/core/runtime/tenant.rs`, finding), true);
  assert.equal(exempt(`${SRC}/storage/mod.rs`, finding), false);
  assert.equal(
    exempt(`${SRC}/core/runtime/spawn.rs`, { rule: "bare-spawn" }),
    true,
  );
  assert.equal(
    exempt(`${SRC}/core/runtime/tenant.rs`, { rule: "bare-spawn" }),
    false,
  );
});

test("a new CoreContext::current() in a tenant-keyed path fails the ratchet", () => {
  const files = {
    [`${SRC}/web_chat/ops/state.rs`]: "let t = current_tenant();\n",
    [`${SRC}/core/runtime/context.rs`]: "let c = CoreContext::current();\n",
  };
  const clean = run(files, []);
  assert.equal(clean.status, 0, clean.out);

  const regressed = run(
    {
      ...files,
      [`${SRC}/web_chat/ops/state.rs`]:
        "let agent = CoreContext::current().and_then(|c| c.session_agent().map(str::to_owned));\n",
    },
    [],
  );
  assert.equal(regressed.status, 1);
  assert.match(
    regressed.out,
    /ambient-context: crates\/openhuman-core\/src\/web_chat\/ops\/state.rs:1/,
  );
  assert.match(regressed.out, /current_tenant/);
});
