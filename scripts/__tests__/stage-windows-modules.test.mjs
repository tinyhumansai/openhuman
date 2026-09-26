import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { readRegistrySource } from "../ci/self-hosted/test-module-assets.mjs";
import { parseAllList } from "../lib/module-pins.mjs";
import { bundledAssets, extractWindowsZip } from "../release/stage-windows-modules.mjs";

test("every compiled module has one pinned Windows installer asset", () => {
  const source = readRegistrySource();
  const assets = bundledAssets(source);
  assert.equal(assets.length, parseAllList(source).length);
  assert.equal(new Set(assets.map((asset) => asset.id)).size, assets.length);
  for (const asset of assets) {
    assert.equal(asset.hostKey, "windows-2022-x86_64");
    assert.match(asset.url, /^https:\/\/github\.com\/tinyhumansai\//);
    assert.match(asset.archive, /-windows-2022-x86_64\.zip$/);
    assert.match(asset.sha256, /^[a-f0-9]{64}$/);
  }
});

test("a missing Windows pin fails staging before any download", () => {
  const source = readRegistrySource().replace(
    'host_key: "windows-2022-x86_64"',
    'host_key: "removed-windows-asset"',
  );
  assert.throws(() => bundledAssets(source), /has no windows-2022-x86_64 asset/);
});

test("Windows extraction rejects an archive entry escaping its destination", {
  skip: process.platform !== "win32",
}, () => {
  const root = mkdtempSync(join(tmpdir(), "openhuman-zip-boundary-"));
  const destination = join(root, "modules");
  mkdirSync(destination);
  const archive = join(root, "escape.zip");
  const name = Buffer.from("../escape.txt");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(20, 6);
  central.writeUInt16LE(name.length, 28);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(local.length + name.length, 16);
  writeFileSync(archive, Buffer.concat([local, name, central, name, end]));

  assert.throws(
    () => extractWindowsZip(archive, destination),
    (error) => /outside/i.test(error.stderr?.toString() ?? ""),
  );
  assert.throws(() => readFileSync(join(root, "escape.txt")), { code: "ENOENT" });
});
