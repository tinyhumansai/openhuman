// Self-test for scripts/ci/check-ci-module-source.mjs (openhuman#7604).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { check, scan } from "../ci/check-ci-module-source.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const CLI = join(REPO_ROOT, "scripts/ci/check-ci-module-source.mjs");

test("the real tree passes", () => {
  assert.deepEqual(check(REPO_ROOT), []);
});

test("the global and per-module env vars are flagged", () => {
  for (const line of [
    "  OPENHUMAN_MODULE_SOURCE: linked",
    "export OPENHUMAN_MODULE_SOURCE=source",
    "env: { OPENHUMAN_MODULE_SOURCE_TINYJUICE: source }",
  ])
    assert.equal(scan(line, "w.yml").length, 1, line);
});

test("linked and source overrides are flagged, release is not", () => {
  assert.equal(scan('source = "linked"', "c.toml").length, 1);
  assert.equal(scan('{"source": "source"}', "c.json").length, 1);
  assert.equal(scan('source = "release"', "c.toml").length, 0);
  assert.equal(scan('const source = "linkedin"', "x.mjs").length, 0);
});

test("enabling the linked-modules feature is flagged", () => {
  assert.equal(
    scan("cargo test --features linked-modules", "w.yml").length,
    1,
  );
});

test("running the local module build script from CI is flagged", () => {
  const found = scan("run: node scripts/modules/build.mjs all", "w.yml");
  assert.deepEqual(
    found.map((v) => v.rule),
    ["module-build-script"],
  );
  assert.equal(scan("# node scripts/modules/build.mjs all", "w.yml").length, 0);
});

test("comments may explain the rule", () => {
  assert.deepEqual(
    scan("# never set OPENHUMAN_MODULE_SOURCE in CI", "w.yml"),
    [],
  );
});

test("the CLI exits non-zero on a violating workflow and zero on a clean one", () => {
  const root = mkdtempSync(join(tmpdir(), "ci-module-source-"));
  try {
    mkdirSync(join(root, ".github/workflows"), { recursive: true });
    mkdirSync(join(root, "scripts/ci"), { recursive: true });
    writeFileSync(join(root, ".github/workflows/ok.yml"), "run: cargo test\n");
    let result = spawnSync(process.execPath, [CLI, root], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    writeFileSync(
      join(root, ".github/workflows/bad.yml"),
      "env:\n  OPENHUMAN_MODULE_SOURCE: linked\n",
    );
    result = spawnSync(process.execPath, [CLI, root], { encoding: "utf8" });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /bad\.yml:2/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
