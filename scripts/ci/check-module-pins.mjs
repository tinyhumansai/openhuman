#!/usr/bin/env node
// Fails when a module's registry pin and its submodule pin describe different
// releases — the drift behind openhuman#5727.
//
// Nine subsystems load as downloaded cdylib modules, and each is pinned TWICE,
// independently: once as a git submodule (the source this repo compiles the
// wire contract against) and once as a `version` + per-platform SHA-256 in
// `src/openhuman/modules/registry.rs` (the artifact actually loaded at runtime).
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
// Three things are checked, and the third is the one that generalises:
//
//   1. Every record in `ALL` sits on the tag its `version` names, unless it is
//      declared in module-pin-exemptions.json with the exact drift it has.
//   2. For tinymemory — the only module pinned in more than two places — the
//      registry version, `ARTIFACT_CAPABILITIES_PIN`, and every `memory_version`
//      / `memory_sha256` pair in the workflows all describe one release.
//   3. Every record in `ALL` is accounted for by the pin map below. A module
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
  expandRustIncludes,
  parseAllList,
  parseArtifactCapabilitiesPin,
  parseRecords,
  parseWorkflowMemoryBlocks,
} from "../lib/module-pins.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(process.argv[2] ?? join(HERE, "..", ".."));

// Record id -> the submodule that is the source of truth for its version.
//
// `submodule: null` means "this record has no submodule of its own", and needs a
// reason. It is NOT an exemption from drift — the two runtime providers below
// ship out of the tinyruntime release, so they are checked against
// `vendor/tinyruntime` via `sharesWith`.
const PIN_MAP = {
  tinydocs: { submodule: "vendor/tinydocs" },
  tinywallet: { submodule: "vendor/tinywallet" },
  tinymemory: { submodule: "vendor/tinymemory" },
  tinyjuice: { submodule: "vendor/tinyjuice" },
  tinyvoice: { submodule: "vendor/tinyvoice" },
  tinyruntime: { submodule: "vendor/tinyruntime" },
  tinymcp: { submodule: "vendor/tinymcp" },
  tinyconnectors: { submodule: "vendor/tinyconnectors" },
  "tinyruntime-nodejs": {
    submodule: null,
    sharesWith: "vendor/tinyruntime",
    reason: "published from the tinyruntime release; no repository of its own",
  },
  "tinyruntime-python": {
    submodule: null,
    sharesWith: "vendor/tinyruntime",
    reason: "published from the tinyruntime release; no repository of its own",
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

// ── Parse the registry ────────────────────────────────────────────────────────

const registrySrc = readRustModule(
  "src/openhuman/modules/registry.rs",
  "registry",
);
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

// ── Check 3 first: is every record accounted for? ─────────────────────────────
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
    ["describe", "--tags", "HEAD"],
    abs,
    `describe ${submodulePath}`,
  );
}

const describeCache = new Map();
function describeCached(path) {
  if (!describeCache.has(path)) describeCache.set(path, describe(path));
  return describeCache.get(path);
}

for (const rec of active) {
  const entry = PIN_MAP[rec.id];
  if (!entry) continue; // already reported above
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

// ── Check 2: the tinymemory pin set ───────────────────────────────────────────
//
// tinymemory is the only record pinned in more than two places, so it is the
// only one with a spread this wide. Keyed off the record rather than a literal,
// so re-pinning tinymemory does not need this file edited.

const memRec = active.find((r) => r.id === "tinymemory");
if (!memRec) {
  fail(
    'modules::registry::ALL no longer has a "tinymemory" record; the tinymemory pin-set check cannot run',
  );
} else {
  const memSrc = readRustModule(
    "src/openhuman/modules/memory.rs",
    "modules/memory.rs",
  );
  const pin = parseArtifactCapabilitiesPin(memSrc);
  if (!pin) {
    fail(
      "src/openhuman/modules/memory.rs: could not find ARTIFACT_CAPABILITIES_PIN",
    );
  } else if (pin !== memRec.version) {
    fail(
      `ARTIFACT_CAPABILITIES_PIN and the tinymemory registry record disagree.\n` +
        `    registry.rs version              : ${memRec.version}\n` +
        `    modules/memory.rs PIN            : ${pin}\n` +
        `    These name the release whose capability set the host assumes. Moving one\n` +
        `    without the other is what #5598 looked like from the inside.`,
    );
  }

  const WORKFLOWS = [
    ".github/workflows/ci-full.yml",
    ".github/workflows/ci-lite.yml",
    ".github/workflows/e2e-reusable.yml",
  ];
  let sawAnyBlock = false;
  for (const wf of WORKFLOWS) {
    const src = readOrDie(join(ROOT, wf), `workflow ${wf}`);
    const { versions, digests, archives } = parseWorkflowMemoryBlocks(src);

    if (versions.length === 0) {
      // The blocks are how CI gets a real module to test against. If one is
      // renamed away this check must not quietly cover fewer files.
      fail(
        `${wf}: no \`memory_version="…"\` block found. If these moved, update WORKFLOWS in this script.`,
      );
      continue;
    }
    if (versions.length !== digests.length) {
      fail(
        `${wf}: ${versions.length} memory_version block(s) but ${digests.length} memory_sha256 — they pair up`,
      );
    }
    sawAnyBlock = true;

    versions.forEach((v, i) => {
      if (v !== memRec.version) {
        fail(
          `${wf}: memory_version="${v}" but the tinymemory registry record is ${memRec.version}.\n` +
            `    CI would fetch a different release than the product pins.`,
        );
      }
      const digest = digests[i];
      if (!digest) return;
      const known = memRec.assets.find((a) => a.sha256 === digest);
      if (!known) {
        fail(
          `${wf}: memory_sha256="${digest}" matches no asset digest in the tinymemory record.\n` +
            `    Take it verbatim from the release's checksum.toml, as registry.rs:23-25 requires.`,
        );
      }
    });

    // The archive name embeds the version through ${memory_version}; assert the
    // literal half resolves to an asset the record actually publishes.
    for (const a of archives) {
      const resolved = a.replace(/\$\{memory_version\}/g, memRec.version);
      if (!memRec.assets.some((asset) => asset.archive === resolved)) {
        fail(
          `${wf}: builds archive name "${resolved}", which the tinymemory record does not publish.\n` +
            `    Known: ${memRec.assets.map((x) => x.archive).join(", ")}`,
        );
      }
    }
  }
  if (!sawAnyBlock)
    fail(
      "no workflow carried a memory_version block — this check scanned nothing",
    );
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
