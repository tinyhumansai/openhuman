// Pure parsing and decision logic for the module-pin gates (openhuman#5727).
//
// Everything here is a pure function over strings so it can be tested without a
// git tree; the two CLIs in scripts/ci/ own the git calls and the process exit.
// See scripts/ci/check-module-pins.mjs for what the gate is for.

import { posix } from "node:path";

/**
 * Read a Rust module and the source fragments it brings in with `include!`.
 *
 * Large registry and module-client files are split into sibling fragments to
 * keep source files reviewable. The pin gate still needs to inspect the module
 * as Rust sees it, not just its small entrypoint. Keeping the reader callback
 * injectable preserves this module's pure, fixture-friendly design.
 */
export function expandRustIncludes(entryPath, readSource, seen = new Set()) {
  const normalized = posix.normalize(entryPath);
  if (seen.has(normalized)) {
    throw new Error(`recursive Rust include detected at ${normalized}`);
  }

  const nextSeen = new Set(seen).add(normalized);
  const source = readSource(normalized);
  const directory = posix.dirname(normalized);
  const included = [...source.matchAll(/include!\(\s*"([^"]+)"\s*\);/g)].map(
    (match) =>
      expandRustIncludes(posix.join(directory, match[1]), readSource, nextSeen),
  );
  return [source, ...included].join("\n");
}

/**
 * The record names listed in `pub const ALL`, in order.
 *
 * Line comments are stripped BEFORE splitting on commas, not filtered out after.
 * Splitting first puts a standalone `// …` line and the record beneath it in the
 * SAME token, so filtering tokens that begin with `//` discarded the record with
 * the comment. `ALL` carries no comments today, so this was latent rather than
 * live — but the records it lists are heavily commented individually, the list is
 * where a "why is this one eager" note would naturally go, and a dropped record
 * reads downstream as "missing from PIN_MAP": the gate would fail CI while
 * pointing at the wrong thing entirely. Raised by CodeRabbit on #5812.
 */
export function parseAllList(src) {
  const block = src.match(
    /pub const ALL: &\[ModuleRecord\] = &\[([\s\S]*?)\];/,
  );
  if (!block) throw new Error("registry.rs: could not find `pub const ALL`");
  return block[1]
    .replace(/\/\/[^\n]*/g, "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Every `const NAME: ModuleRecord = ModuleRecord { … };`, keyed by NAME. */
export function parseRecords(src) {
  const records = new Map();
  const re =
    /(?:pub )?const ([A-Z0-9_]+): ModuleRecord = ModuleRecord \{([\s\S]*?)\n\};/g;
  for (const m of src.matchAll(re)) {
    const [, name, body] = m;
    const field = (f) => {
      const hit = body.match(new RegExp(`^\\s*${f}: "([^"]*)"`, "m"));
      return hit ? hit[1] : null;
    };
    const assets = [];
    const assetRe =
      /PlatformAsset \{\s*host_key: "([^"]+)",\s*archive: "([^"]+)",\s*sha256: "([^"]+)",\s*\}/g;
    for (const a of body.matchAll(assetRe)) {
      assets.push({ hostKey: a[1], archive: a[2], sha256: a[3] });
    }
    records.set(name, {
      name,
      id: field("id"),
      version: field("version"),
      assets,
    });
  }
  if (records.size === 0)
    throw new Error("registry.rs: parsed zero ModuleRecord blocks");
  return records;
}

/** `ARTIFACT_CAPABILITIES_PIN` from modules/memory.rs. */
export function parseArtifactCapabilitiesPin(src) {
  const m = src.match(/ARTIFACT_CAPABILITIES_PIN: &str = "([^"]+)"/);
  return m ? m[1] : null;
}

/** The `memory_version` / `memory_sha256` / `memory_archive` literals in a workflow. */
export function parseWorkflowMemoryBlocks(src) {
  return {
    versions: [...src.matchAll(/^\s*memory_version="([^"]+)"/gm)].map(
      (m) => m[1],
    ),
    digests: [...src.matchAll(/^\s*memory_sha256="([^"]+)"/gm)].map(
      (m) => m[1],
    ),
    archives: [
      ...src.matchAll(
        /^\s*memory_archive="[^"]*\/(tinymemory-module-[^"]+?)"/gm,
      ),
    ].map((m) => m[1]),
  };
}

/**
 * Decide what one record's pin state means.
 *
 * `actual` is `git describe --tags` output for the submodule; `exemption` is the
 * matching entry from module-pin-exemptions.json, or undefined.
 *
 * Four outcomes, and three of them are failures — including "exempt but the
 * pins now agree". A stale exemption is not harmless: it is a standing licence
 * for the next real drift on that record to pass unnoticed.
 */
export function classifyPin({ id, version, submodulePath, actual, exemption }) {
  const wanted = `v${version}`;
  if (actual === wanted) {
    if (exemption) {
      return {
        ok: false,
        kind: "stale-exemption",
        message:
          `"${id}" is listed in module-pin-exemptions.json, but its pins now AGREE ` +
          `(${submodulePath} is at ${actual}). Delete the exemption — a stale exemption ` +
          `hides the next real drift.`,
      };
    }
    return { ok: true, kind: "match" };
  }
  if (exemption) {
    if (exemption.expect !== actual) {
      return {
        ok: false,
        kind: "exemption-widened",
        message:
          `"${id}" drift has CHANGED since it was accepted.\n` +
          `    registry.rs version : ${version} (expects tag ${wanted})\n` +
          `    ${submodulePath} is at : ${actual}\n` +
          `    exemption pins      : ${exemption.expect}\n` +
          `    An exemption accepts one specific drift, not any drift. If the new state is\n` +
          `    intended, update \`expect\` and the reason in module-pin-exemptions.json.`,
      };
    }
    return { ok: true, kind: "exempt", reason: exemption.reason };
  }
  return {
    ok: false,
    kind: "drift",
    message:
      `"${id}" registry pin and submodule pin describe different releases.\n` +
      `    registry.rs version : ${version}  (would download the v${version} artifact)\n` +
      `    ${submodulePath} is at : ${actual}  (what this build compiles the contract against)\n` +
      `    Fix by moving whichever pin is stale, or — if the two are meant to differ —\n` +
      `    declare it in scripts/ci/module-pin-exemptions.json with a reason.`,
  };
}

/**
 * Every record in `ALL` must be known to the pin map, and every pin-map entry
 * must still be a record.
 *
 * This is the half that survives the next module landing. Six vendored crates
 * were added between 2026-08-20 and 2026-08-27; a gate that enumerated the
 * modules of the day would already be behind, and would report a clean scan
 * while ignoring the newest one.
 */
export function checkPinMapCoverage(activeIds, pinMapIds) {
  const failures = [];
  for (const id of activeIds) {
    if (!pinMapIds.includes(id)) {
      failures.push(
        `modules::registry::ALL contains "${id}", which scripts/ci/check-module-pins.mjs ` +
          `does not know about.\n` +
          `    Add it to PIN_MAP: either name the vendor/ submodule that is the source of\n` +
          `    truth for its version, or set submodule: null with a reason (and sharesWith,\n` +
          `    if it ships out of another crate's release).\n` +
          `    This is deliberate: a new module must not inherit an unchecked pin.`,
      );
    }
  }
  for (const id of pinMapIds) {
    if (!activeIds.includes(id)) {
      failures.push(
        `PIN_MAP has an entry for "${id}", which is no longer in modules::registry::ALL. Remove it.`,
      );
    }
  }
  return failures;
}

/** path -> gitlink sha, parsed from `git ls-tree -r <ref>` output. */
export function parseGitlinks(lsTreeOutput) {
  const map = new Map();
  for (const line of lsTreeOutput.split("\n")) {
    const m = line.match(/^160000 commit ([0-9a-f]{40})\t(.+)$/);
    if (m) map.set(m[2], m[1]);
  }
  return map;
}

/** A deliberate rewind is declared on the branch that does it. */
export function rewindDeclared(text) {
  return /\[pin-rewind\]/i.test(text ?? "");
}

/**
 * Classify a submodule pin move from BOTH ancestry answers.
 *
 * "Not an ancestor" is not the same as "forward". When the new commit and the
 * base commit are siblings — a side branch, or a submodule whose history was
 * rebased — neither is an ancestor of the other, and a check that asks only
 * `is-ancestor head base` sees its single query fail and calls the move forward.
 *
 * A divergent pin drops whatever the base branch had just as surely as a rewind
 * does; it simply does it sideways. Since the whole point of this gate is that a
 * pin must not silently lose commits, treating divergence as forward would
 * defeat it: a rewind onto a sibling — 14a23b994's shape, one rebase away —
 * would walk straight through.
 *
 * So both directions are asked and only a genuine descendant is forward.
 */
export function classifyMove({ headIsAncestorOfBase, baseIsAncestorOfHead }) {
  if (headIsAncestorOfBase) return "rewind";
  if (baseIsAncestorOfHead) return "forward";
  return "divergent";
}

/**
 * Does `toplevel` prove that `path` is a checked-out submodule?
 *
 * `git -C <path> rev-parse --git-dir` cannot answer this: git walks UPWARD, so
 * from an uninitialised or merely empty `vendor/<x>` it answers the
 * SUPERPROJECT's `.git` and exits 0 — reporting "present" having checked
 * nothing. `--show-toplevel` answers the root of whichever repository owns the
 * directory, and only when that root IS the directory is it a real checkout.
 *
 * Both sides must already be realpath-resolved by the caller: macOS resolves
 * `/tmp/...` to `/private/tmp/...`, and comparing one resolved path against one
 * unresolved path is its own quiet false negative.
 */
export function toplevelProvesSubmodule(resolvedToplevel, resolvedPath) {
  return Boolean(resolvedToplevel) && resolvedToplevel === resolvedPath;
}
