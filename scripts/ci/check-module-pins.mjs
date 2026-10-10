#!/usr/bin/env node
// Fails when a module's registry pin and its submodule pin describe different
// releases — the drift behind openhuman#5727.
//
// Downloaded cdylib modules are pinned TWICE,
// independently: once as a git submodule (the source this repo compiles the
// wire contract against) and once as a `version` + per-platform SHA-256 in
// `crates/openhuman-core/src/modules/registry.rs` (the artifact actually loaded at runtime).
// Nothing compared the two. When they drift, the build compiles clean, every
// lane is green, and a capability goes missing at runtime on a user's machine —
// which is how #5598 (capability bitmask 8191 vs 262143), #5623 (missing
// ListAllFacets) and #5641 (missing profile family) each shipped. All three were
// a stale pin; none left behind anything that would catch the fourth.
//
// The module ABI gate cannot catch it: it checks magic, ABI revision, pointer
// width, target triple, endianness, feature bits and panic strategy — none of
// which encodes WHICH RELEASE the artifact is. A correctly-built older artifact
// is admitted without complaint.
//
// Two things are checked, and the second is the one that generalises:
//
//   1. Every record in `ALL` sits on the tag its `version` names, unless it is
//      declared in module-pin-exemptions.json with the exact drift it has.
//   2. Every record in `ALL` is accounted for by the pin map below. A module
//      added without a decision here fails the gate rather than silently
//      escaping it. Six vendored crates landed between 2026-08-20 and -27; a
//      guard that enumerated today's modules would already be behind.
//
// This is deliberately NOT the whole story. A pin can satisfy every check here
// and still be wrong, because a pin that is internally consistent can be
// consistent with an OLDER release — see scripts/ci/check-submodule-monotonic.mjs
// for the other half.
//
// No network. Requires submodules to be checked out; if one is missing this
// FAILS rather than skipping (see `mustRun`). We have twice shipped a gate that
// swallowed a git error and reported clean having scanned nothing.
//
// Usage: check-module-pins.mjs [repo-root]
import { readFileSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";

// Any throw below is a check that could not be COMPLETED, which is not the same
// as a check that passed. Report it as a failure with a legible message rather
// than a stack trace, and never exit 0. Two gates have shipped here that
// swallowed a git error and reported clean having scanned nothing.
process.on("uncaughtException", (error) => {
  console.error(`\nModule pin check FAILED\n`);
  console.error(`  \u2717 ${error.message}\n`);
  console.error(
    "  This gate could not complete. That is a failure, not a pass.\n",
  );
  process.exit(1);
});
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  checkPinMapCoverage,
  classifyPin,
  classifyProviderPin,
  expandRustIncludes,
  parseAllList,
  parseRecords,
} from "../lib/module-pins.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(process.argv[2] ?? join(HERE, "..", ".."));

// Record id -> the submodule that is the source of truth for its version.
//
// `submodule: null` means "this record has no submodule of its own", and needs a
// reason. It is NOT an exemption from drift. The two runtime providers below are
// released from their own repositories on their own version line, so their
// version cannot equal a submodule tag; `provider` instead checks them against
// scripts/ci/module-provider-pins.json (their release, and the commit of
// `builtAgainst` that release was built from, which must be the host's pin).
const PIN_MAP = {
  tinysecurity: { submodule: "vendor/tinysecurity" },
  tinysearch: { submodule: "vendor/tinysearch" },
  tinycomputer: { submodule: "vendor/tinycomputer" },
  tinybox: { submodule: "vendor/tinybox" },
  tinychannels: { submodule: "vendor/tinychannels" },
  tinyhosts: { submodule: "vendor/tinyhosts" },
  tinydocs: { submodule: "vendor/tinydocs" },
  tinywallet: { submodule: "vendor/tinywallet" },
  tinyjuice: { submodule: "vendor/tinyjuice" },
  tinyvoice: { submodule: "vendor/tinyvoice" },
  tinyruntime: { submodule: "vendor/tinyruntime" },
  tinymcp: { submodule: "vendor/tinymcp" },
  tinyconnectors: { submodule: "vendor/tinyconnectors" },
  tinybox: { submodule: "vendor/tinybox" },
  tinychannels: { submodule: "vendor/tinychannels" },
  tinyhosts: { submodule: "vendor/tinyhosts" },
  "tinyruntime-nodejs": {
    submodule: null,
    provider: { builtAgainst: "vendor/tinyruntime" },
    reason:
      "released from tinyhumansai/tinyruntime-nodejs on its own version line; checked via module-provider-pins.json",
  },
  "tinyruntime-python": {
    submodule: null,
    provider: { builtAgainst: "vendor/tinyruntime" },
    reason:
      "released from tinyhumansai/tinyruntime-python on its own version line; checked via module-provider-pins.json",
  },
};

const failures = [];
const notes = [];
const fail = (m) => failures.push(m);

/** Run a command, or die. Never returns a sentinel a caller could mistake for success. */
function mustRun(cmd, args, cwd, what) {
  try {
    return execFileSync(cmd, args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch (error) {
    // Fail closed. A gate that treats "could not look" as "nothing wrong" is
    // worse than no gate: it reports a clean scan of nothing.
    const detail = (error.stderr || error.message || "")
      .toString()
      .trim()
      .split("\n")[0];
    throw new Error(
      `${what}: \`${cmd} ${args.join(" ")}\` failed in ${cwd}: ${detail}`,
    );
  }
}

function readOrDie(path, what) {
  if (!existsSync(path))
    throw new Error(`${what}: expected file is missing: ${path}`);
  return readFileSync(path, "utf8");
}

function readRustModule(relativePath, what) {
  return expandRustIncludes(relativePath, (includedPath) =>
    readOrDie(join(ROOT, includedPath), `${what} include`),
  );
}

/**
 * Read the registry entrypoint and every record fragment it declares.
 *
 * The registry deliberately keeps its `ModuleRecord` definitions in small
 * `registry/records_*.rs` siblings. This gate must inspect those definitions,
 * not merely the `ALL` wiring file, or a source-layout refactor turns its
 * security check into an empty scan.
 */
function readRegistry() {
  const relativePath = "crates/openhuman-core/src/modules/registry.rs";
  const entry = readRustModule(relativePath, "registry");
  const recordsDir = relativePath.replace(/\.rs$/, "");
  const recordModules = [
    ...entry.matchAll(/^mod (records_[a-z0-9_]+);$/gm),
  ].map(([, name]) => `${recordsDir}/${name}.rs`);

  return [
    entry,
    ...recordModules.map((path) =>
      readRustModule(path, `registry record ${path}`),
    ),
  ].join("\n");
}

// ── Parse the registry ────────────────────────────────────────────────────────

const registrySrc = readRegistry();
const allNames = parseAllList(registrySrc);
const records = parseRecords(registrySrc);

const active = [];
for (const name of allNames) {
  const rec = records.get(name);
  if (!rec) {
    fail(
      `registry.rs: \`ALL\` lists ${name}, but no such ModuleRecord was parsed`,
    );
    continue;
  }
  if (!rec.id || !rec.version) {
    fail(`registry.rs: ${name} is missing an id or version`);
    continue;
  }
  active.push(rec);
}
if (active.length === 0)
  fail("registry.rs: `ALL` resolved to zero usable records");

// ── Check 2 first: is every record accounted for? ─────────────────────────────
//
// Before checking pins, check that we KNOW about every record. Running the pin
// checks first would report "all pins agree" on a tree containing a module this
// gate has never heard of.
for (const f of checkPinMapCoverage(
  active.map((r) => r.id),
  Object.keys(PIN_MAP),
))
  fail(f);

// ── Exemptions ────────────────────────────────────────────────────────────────

const exemptionsRaw = readOrDie(
  join(HERE, "module-pin-exemptions.json"),
  "exemptions",
);
let exemptions;
try {
  exemptions = JSON.parse(exemptionsRaw).exemptions ?? [];
} catch (error) {
  throw new Error(
    `module-pin-exemptions.json is not valid JSON: ${error.message}`,
  );
}
const exemptById = new Map(exemptions.map((e) => [e.id, e]));
for (const e of exemptions) {
  if (!e.id || !e.expect || !e.reason) {
    fail(
      `module-pin-exemptions.json: entry ${JSON.stringify(e.id ?? e)} needs id, expect and reason`,
    );
  }
  if (!active.some((r) => r.id === e.id)) {
    fail(
      `module-pin-exemptions.json: "${e.id}" is not a record in modules::registry::ALL. Remove it.`,
    );
  }
}

// ── Check 1: registry version vs submodule tag ────────────────────────────────

function describe(submodulePath) {
  const abs = join(ROOT, submodulePath);
  if (!existsSync(join(abs, ".git"))) {
    // Not "skip". A lane that runs this gate must check submodules out; if it
    // does not, the gate has scanned nothing and must say so.
    throw new Error(
      `${submodulePath} is not a checked-out submodule (no .git). This gate needs ` +
        `submodules; run it in a lane with \`submodules: recursive\`.`,
    );
  }
  // `--tags` so lightweight tags count; no `--exact-match`, because the drift we
  // are hunting is exactly the "N commits past the tag" form and we want to
  // report it rather than get an opaque failure.
  return mustRun(
    "git",
    ["describe", "--tags", "--abbrev=8", "HEAD"],
    abs,
    `describe ${submodulePath}`,
  );
}

const describeCache = new Map();
function describeCached(path) {
  if (!describeCache.has(path)) describeCache.set(path, describe(path));
  return describeCache.get(path);
}

function headCommit(submodulePath) {
  describeCached(submodulePath); // fails closed when it is not checked out
  return mustRun(
    "git",
    ["rev-parse", "HEAD"],
    join(ROOT, submodulePath),
    `rev-parse ${submodulePath}`,
  );
}

const providerLocks = JSON.parse(
  readOrDie(join(HERE, "module-provider-pins.json"), "provider pins"),
).providers;

for (const rec of active) {
  const entry = PIN_MAP[rec.id];
  if (!entry) continue; // already reported above
  if (entry.provider) {
    const lock = providerLocks[rec.id];
    const src = entry.provider.builtAgainst;
    const verdict = classifyProviderPin({
      id: rec.id,
      version: rec.version,
      releaseUrl: rec.releaseUrl,
      lock,
      sourceSubmodule: src,
      sourceHead: headCommit(src),
    });
    if (!verdict.ok) fail(verdict.message);
    continue;
  }
  const path = entry.submodule ?? entry.sharesWith;
  if (!path) {
    fail(`PIN_MAP["${rec.id}"] has neither submodule nor sharesWith`);
    continue;
  }
  const actual = describeCached(path);
  const verdict = classifyPin({
    id: rec.id,
    version: rec.version,
    submodulePath: path,
    actual,
    exemption: exemptById.get(rec.id),
  });
  if (!verdict.ok) {
    fail(verdict.message);
  } else if (verdict.kind === "exempt") {
    const why =
      verdict.reason.length > 96
        ? `${verdict.reason.slice(0, 93)}...`
        : verdict.reason;
    notes.push(`  ~ ${rec.id}: ${actual} — declared exemption: ${why}`);
  }
}

// ── Report ────────────────────────────────────────────────────────────────────

if (failures.length > 0) {
  console.error("\nModule pin check FAILED\n");
  for (const f of failures) console.error(`  ✗ ${f}\n`);
  console.error(
    "Why this gate exists: a module is pinned twice — as a submodule (the contract this\n" +
      "build compiles against) and as a version+digest in modules/registry.rs (the artifact\n" +
      "loaded at runtime). When they disagree the build is green and the failure lands on a\n" +
      "user's machine. openhuman#5727.\n",
  );
  process.exit(1);
}

console.log(
  `Module pin check OK — ${active.length} record(s) in modules::registry::ALL, all pins accounted for.`,
);
for (const n of notes) console.log(n);
