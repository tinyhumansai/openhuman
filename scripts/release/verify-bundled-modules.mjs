#!/usr/bin/env node
// Verify that every archive (and macOS digest marker) in a staged
// `bundled-modules/` directory hashes to the sha256 pin baked into the Rust
// module registry at `crates/openhuman-core/src/modules/records_*.rs`.
//
// Why this exists: `stage-modules.mjs` already re-hashes each archive right
// after download, so a mismatch there is impossible on a fresh stage. But in
// production (TAURI-RUST-11MZ / issue #8500) tinybus has refused the bundled
// archive on 2000+ events on 0.64.15, with the exact error
//
//   module 'tinyconnectors' could not be loaded from the installer bundle:
//   module `github-release` refused: release is not cached and downloads are
//   disabled. Restart the app after repairing the installation
//
// which is what the tinybus loader emits when a bundled archive is on disk
// but its bytes do NOT match the compiled-in `expected_sha256`. The gap that
// stage-modules alone cannot close is pin drift BETWEEN staging and
// compilation (someone re-pinned the record after the staged dir was
// generated, or a cached stage from an earlier pin shipped alongside a
// freshly compiled binary). This script makes a mismatch impossible to ship:
// run it after `stage-modules.mjs` and before `cargo tauri build`, and run
// it one more time against the final installer artifacts.
//
// Usage: node scripts/release/verify-bundled-modules.mjs [--dir DIR]
//                                                        [--target TRIPLE | --host-key KEY]
//
// Without `--target`/`--host-key` the gate only checks that every present
// staged entry matches its pin — it cannot catch an incomplete stage (a tree
// with one correctly-pinned macOS marker and no Linux archives would pass on
// a Linux build). Passing the build target (as the matrix step does) turns
// on the completeness check: every asset the registry expects for that host
// MUST also be on disk, or the gate reports it as missing.
//
// Exits 0 if every staged file matches its pin and (when a target is given)
// the expected asset set is complete; non-zero with a human-readable diff on
// the first miss.

import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseAllList, parseRecords } from "../lib/module-pins.mjs";
import {
  readRegistrySource,
} from "../ci/self-hosted/test-module-assets.mjs";
import {
  bundledAssets,
  hostKeyForTarget,
  keepsArchive,
} from "./stage-modules.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DEFAULT_DIR = join(ROOT, "crates/openhuman-app/bundled-modules");

function option(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? undefined : process.argv[index + 1];
}

function listDirs(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name);
}

function sha256OfFile(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function readMarker(path) {
  return readFileSync(path, "utf8").trim();
}

/**
 * Build the expected-pin index from the Rust registry: a Map keyed by
 * `${id}/${version}/${hostKey}/${archive}` → `expected_sha256`.
 */
function pinsIndex(source) {
  const names = parseAllList(source);
  const records = parseRecords(source);
  const index = new Map();
  for (const name of names) {
    const r = records.get(name);
    if (!r) throw new Error(`records missing for ${name}`);
    for (const a of r.assets) {
      index.set(`${r.id}/${r.version}/${a.hostKey}/${a.archive}`, a.sha256.toLowerCase());
    }
  }
  return index;
}

/**
 * Walk a staged bundled-modules tree and yield one entry per archive
 * (or marker) found: `{ id, version, hostKey, archive, path, kind }`.
 * `kind` is `"archive"` or `"marker"`.
 */
function* walkStaged(root) {
  for (const id of listDirs(root)) {
    const idDir = join(root, id);
    for (const version of listDirs(idDir)) {
      const versionDir = join(idDir, version);
      for (const hostKey of listDirs(versionDir)) {
        const hostDir = join(versionDir, hostKey);
        for (const entry of readdirSync(hostDir)) {
          const path = join(hostDir, entry);
          if (!statSync(path).isFile()) continue;
          if (entry.endsWith(".sha256")) {
            yield {
              id,
              version,
              hostKey,
              archive: entry.slice(0, -".sha256".length),
              path,
              kind: "marker",
            };
          } else if (/\.(tar\.gz|zip)$/.test(entry)) {
            yield { id, version, hostKey, archive: entry, path, kind: "archive" };
          }
        }
      }
    }
  }
}

/**
 * The expected (id, version, hostKey, archive) key set for `hostKey`,
 * derived the same way `stage-modules.mjs` picks what to download.
 */
function expectedKeySet(source, hostKey) {
  return new Set(
    bundledAssets(source, hostKey).map(
      (a) => `${a.id}/${a.version}/${a.hostKey}/${a.archive}`,
    ),
  );
}

export function verifyBundledModules({
  dir = DEFAULT_DIR,
  source,
  hostKey,
} = {}) {
  const registrySource = source ?? readRegistrySource();
  const expected = pinsIndex(registrySource);
  const mustStage = hostKey
    ? expectedKeySet(registrySource, hostKey)
    : null;

  const problems = [];
  let checked = 0;
  for (const entry of walkStaged(dir)) {
    checked += 1;
    const key = `${entry.id}/${entry.version}/${entry.hostKey}/${entry.archive}`;
    const pin = expected.get(key);
    if (!pin) {
      problems.push({
        kind: "unknown",
        entry,
        message:
          `staged file has no matching registry pin: ${key} (compiled registry does not list this id/version/hostKey/archive)`,
      });
      continue;
    }
    // `walkStaged` strips `.sha256` from a marker so an archive and a marker
    // on the same hostKey produce the same `key` — a stray marker on a Linux
    // or Windows host would otherwise satisfy both the pin lookup and the
    // completeness check for an asset that stage-modules.mjs requires to be
    // a real archive. `keepsArchive(hostKey)` is the single source of truth
    // for which host carries which kind on disk; run the type check here
    // before anything can accept the entry (and before completeness clears
    // the key from `mustStage`).
    const expectedKind = keepsArchive(entry.hostKey) ? "archive" : "marker";
    if (entry.kind !== expectedKind) {
      problems.push({
        kind: "wrong-type",
        entry,
        message:
          `staged ${entry.kind} is not the required ${expectedKind} for ${entry.hostKey}: ${key} ` +
          `(stage-modules.mjs produces archives on hosts where keepsArchive(hostKey) is true, markers otherwise)`,
      });
      continue;
    }
    if (mustStage) mustStage.delete(key);
    const actual =
      entry.kind === "archive" ? sha256OfFile(entry.path) : readMarker(entry.path).toLowerCase();
    if (actual !== pin) {
      problems.push({
        kind: "mismatch",
        entry,
        expected: pin,
        actual,
        message:
          `pin drift for ${key}: registry expects ${pin}, staged ${entry.kind} is ${actual}. ` +
          `This is the exact condition that fires TAURI-RUST-11MZ / issue #8500 ` +
          `("release is not cached and downloads are disabled") at module load.`,
      });
    }
  }

  if (mustStage) {
    for (const key of mustStage) {
      problems.push({
        kind: "missing",
        key,
        hostKey,
        message:
          `staged tree is missing an asset the registry requires for ${hostKey}: ${key}. ` +
          `A partial stage ships an installer whose host-side module set is incomplete.`,
      });
    }
  }

  if (checked === 0) {
    throw new Error(
      `no archives or markers under ${dir}; was \`stage-modules.mjs\` run?`,
    );
  }

  return { checked, problems };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = option("--dir") ? resolve(option("--dir")) : DEFAULT_DIR;
  const target = option("--target");
  const hostKey = option("--host-key") ?? (target ? hostKeyForTarget(target) : undefined);
  const { checked, problems } = verifyBundledModules({ dir, hostKey });
  if (problems.length === 0) {
    const scope = hostKey ? ` for ${hostKey}` : "";
    console.log(`[verify-bundled-modules] ok: ${checked} entries match their registry pins${scope}`);
    process.exit(0);
  }
  console.error(`[verify-bundled-modules] FAIL: ${problems.length} problem(s) across ${checked} entries:`);
  for (const p of problems) console.error(`  - ${p.message}`);
  process.exit(1);
}
