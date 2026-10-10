#!/usr/bin/env node
// Resolve the native test-module release assets the Rust coverage lane needs,
// straight from the compiled module registry (the authoritative pin).
//
// Usage: node scripts/ci/self-hosted/test-module-assets.mjs [host_key]
// Prints one line per module: `<id>\t<url>\t<archive>\t<sha256>`.
//
// Reading the registry instead of copying version + digest into a workflow
// provides the authoritative release asset metadata for test fixtures.
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { parseRecords } from "../../lib/module-pins.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const REGISTRY_DIR = join(ROOT, "crates/openhuman-core/src/modules");

/** Test modules the core suite loads, by registry record id. */
export const TEST_MODULE_IDS = ["tinyjuice", "tinysearch"];

/** Concatenate registry.rs and its `registry/records_*.rs` fragments. */
export function readRegistrySource(dir = REGISTRY_DIR) {
  const fragments = readdirSync(join(dir, "registry"))
    .filter((f) => /^records_.*\.rs$/.test(f))
    .sort()
    .map((f) => readFileSync(join(dir, "registry", f), "utf8"));
  return [readFileSync(join(dir, "registry.rs"), "utf8"), ...fragments].join(
    "\n",
  );
}

/** `release_url` per record id, from the same source text. */
export function parseReleaseUrls(src) {
  const urls = new Map();
  // Keep fields inside their record. A global regex can skip an empty URL
  // and accidentally associate the following module's release with this id.
  for (const record of parseRecords(src).values()) {
    if (record.id && record.releaseUrl && !urls.has(record.id)) {
      urls.set(record.id, record.releaseUrl);
    }
  }
  return urls;
}

/** Resolve `{id, url, archive, sha256}` for each wanted module on `hostKey`. */
export function resolveTestModuleAssets(src, hostKey, ids = TEST_MODULE_IDS) {
  const byId = new Map([...parseRecords(src).values()].map((r) => [r.id, r]));
  const releaseUrls = parseReleaseUrls(src);
  return ids.map((id) => {
    const record = byId.get(id);
    if (!record)
      throw new Error(`registry has no ModuleRecord with id "${id}"`);
    const asset = record.assets.find((a) => a.hostKey === hostKey);
    if (!asset)
      throw new Error(`${id} ${record.version} has no ${hostKey} asset`);
    const release = releaseUrls.get(id);
    if (!release || !release.includes("/releases/tag/")) {
      throw new Error(`${id}: release_url missing or not a /releases/tag/ URL`);
    }
    const url = `${release.replace("/releases/tag/", "/releases/download/")}/${asset.archive}`;
    return { id, url, archive: asset.archive, sha256: asset.sha256 };
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const hostKey = process.argv[2] ?? "ubuntu-22.04-x86_64";
  for (const a of resolveTestModuleAssets(readRegistrySource(), hostKey)) {
    console.log([a.id, a.url, a.archive, a.sha256].join("\t"));
  }
}
