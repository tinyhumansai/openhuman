import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, "..", "verify-i18n-bundle.mjs");

function run(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    encoding: "utf8",
  });
}

test("verify-i18n-bundle --dist rejects a missing path before filesystem checks", () => {
  const result = run(["--dist"]);

  assert.equal(result.status, 2, result.stderr);
  assert.match(result.stderr, /--dist requires a path/);
  assert.doesNotMatch(result.stderr, /dist directory does not exist/);
});

test("verify-i18n-bundle --dist rejects another flag as the path value", () => {
  const result = run(["--dist", "--help"]);

  assert.equal(result.status, 2, result.stderr);
  assert.match(result.stderr, /--dist requires a path/);
  assert.doesNotMatch(result.stderr, /dist directory does not exist/);
});

function withBundle(content, assertion) {
  const dir = mkdtempSync(join(tmpdir(), "openhuman-i18n-bundle-"));
  try {
    writeFileSync(join(dir, "app.js"), content);
    assertion(run(["--dist", dir]));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("verify-i18n-bundle accepts the Japanese picker and dictionary markers", () => {
  withBundle("zh-CN 简体中文 日本語 言語", (result) => {
    assert.equal(result.status, 0, result.stderr);
  });
});

test("verify-i18n-bundle rejects a build missing Japanese translations", () => {
  withBundle("zh-CN 简体中文 日本語", (result) => {
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /Japanese language setting translation/);
  });
});

test("verify-i18n-bundle accepts escaped Japanese markers", () => {
  withBundle(
    String.raw`zh-CN 简体中文 \u65e5\u672c\u8a9e \u8a00\u8a9e`,
    (result) => {
      assert.equal(result.status, 0, result.stderr);
    },
  );
});
