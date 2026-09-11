// Tests for the module-pin gates (openhuman#5727).
//
// The pure logic is covered directly; the two CLIs are driven end-to-end against
// the real tree, including the two commits that actually shipped a regression.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

// Every name below is the SHIPPED implementation from scripts/lib/module-pins.mjs
// — the same module the two CLIs in scripts/ci/ import. Nothing in this file
// re-implements any of it, so a regression in the lib fails these tests rather
// than passing against a local copy.
import {
  checkPinMapCoverage,
  classifyMove,
  classifyPin,
  expandRustIncludes,
  parseAllList,
  parseArtifactCapabilitiesPin,
  parseGitlinks,
  parseRecords,
  parseWorkflowMemoryBlocks,
  rewindDeclared,
  toplevelProvesSubmodule,
} from "../lib/module-pins.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const PINS_CLI = join(REPO_ROOT, "scripts/ci/check-module-pins.mjs");
const MONO_CLI = join(REPO_ROOT, "scripts/ci/check-submodule-monotonic.mjs");
const readRustModule = (relativePath) =>
  expandRustIncludes(relativePath, (includedPath) =>
    readFileSync(join(REPO_ROOT, includedPath), "utf8"),
  );

const run = (cli, args = [], env = {}) =>
  spawnSync(process.execPath, [cli, ...args], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });

/**
 * Is `vendor/tinymemory` really a checked-out submodule?
 *
 * NOT `rev-parse --git-dir`: git walks UPWARD, so from an uninitialised — or
 * merely empty — `vendor/tinymemory` that command happily answers the
 * SUPERPROJECT's `.git` and exits 0. The probe reported "present" having checked
 * nothing, enabling a case whose CLI then failed for precisely the reason the
 * probe was meant to detect. That is the fail-open shape this whole gate exists
 * to prevent, reproduced inside its own test helper. Raised by Codex on #5812.
 *
 * `--show-toplevel` is the honest question: it answers the root of whichever
 * repository owns that directory. Only when that root IS the submodule path is
 * the submodule genuinely checked out. Compared through `realpathSync` because
 * macOS resolves `/tmp/...` to `/private/tmp/...`.
 */
function submodulesPresent() {
  const path = join(REPO_ROOT, "vendor/tinymemory");
  try {
    const top = execFileSync(
      "git",
      ["-C", path, "rev-parse", "--show-toplevel"],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    ).trim();
    return toplevelProvesSubmodule(realpathSync(top), realpathSync(path));
  } catch {
    return false;
  }
}

// ── Registry parsing ──────────────────────────────────────────────────────────

test("parses every record `ALL` lists out of the real registry", () => {
  const src = readRustModule("src/openhuman/modules/registry.rs");
  const names = parseAllList(src);
  const records = parseRecords(src);
  assert.ok(
    names.length >= 9,
    `expected at least 9 records in ALL, got ${names.length}`,
  );
  for (const name of names) {
    const rec = records.get(name);
    assert.ok(rec, `ALL lists ${name} but no such ModuleRecord parsed`);
    assert.ok(rec.id, `${name} has no id`);
    assert.match(
      rec.version,
      /^\d+\.\d+\.\d+$/,
      `${name} version looks wrong: ${rec.version}`,
    );
  }
});

test("parses per-platform assets, digests included", () => {
  const src = readRustModule("src/openhuman/modules/registry.rs");
  const mem = [...parseRecords(src).values()].find(
    (r) => r.id === "tinymemory",
  );
  assert.ok(mem, "tinymemory record not found");
  assert.ok(mem.assets.length > 0, "tinymemory publishes no assets?");
  for (const a of mem.assets) {
    assert.match(a.sha256, /^[0-9a-f]{64}$/, `bad digest for ${a.hostKey}`);
    assert.ok(
      a.archive.includes(mem.version),
      `${a.archive} does not carry version ${mem.version}`,
    );
  }
});

test("reading a registry with no ALL block throws rather than returning empty", () => {
  assert.throws(() => parseAllList("fn main() {}"), /could not find/);
  assert.throws(() => parseRecords("fn main() {}"), /parsed zero/);
});

test("finds ARTIFACT_CAPABILITIES_PIN and the workflow memory blocks", () => {
  const memSrc = readRustModule("src/openhuman/modules/memory.rs");
  assert.match(parseArtifactCapabilitiesPin(memSrc), /^\d+\.\d+\.\d+$/);
  const wf = readFileSync(
    join(REPO_ROOT, ".github/workflows/ci-lite.yml"),
    "utf8",
  );
  const blocks = parseWorkflowMemoryBlocks(wf);
  assert.ok(
    blocks.versions.length > 0,
    "ci-lite.yml has no memory_version block",
  );
  assert.equal(
    blocks.versions.length,
    blocks.digests.length,
    "version/digest blocks must pair",
  );
});

// ── Classification ────────────────────────────────────────────────────────────

const base = { id: "tinyx", version: "1.2.3", submodulePath: "vendor/tinyx" };

test("a pin sitting on its tag passes", () => {
  const v = classifyPin({ ...base, actual: "v1.2.3", exemption: undefined });
  assert.equal(v.ok, true);
  assert.equal(v.kind, "match");
});

test("a pin past its tag fails and names both sides", () => {
  const v = classifyPin({
    ...base,
    actual: "v1.2.3-7-gabc1234",
    exemption: undefined,
  });
  assert.equal(v.ok, false);
  assert.equal(v.kind, "drift");
  assert.match(v.message, /1\.2\.3/);
  assert.match(v.message, /v1\.2\.3-7-gabc1234/);
});

test("a declared exemption passes only for the exact drift it declares", () => {
  const exemption = {
    id: "tinyx",
    expect: "v1.2.3-7-gabc1234",
    reason: "because",
  };
  const same = classifyPin({ ...base, actual: "v1.2.3-7-gabc1234", exemption });
  assert.equal(same.ok, true);
  assert.equal(same.kind, "exempt");

  // Drifting FURTHER is a new fact, and must be re-declared.
  const wider = classifyPin({
    ...base,
    actual: "v1.2.3-9-gdef5678",
    exemption,
  });
  assert.equal(wider.ok, false);
  assert.equal(wider.kind, "exemption-widened");
});

test("an exemption whose drift has been fixed fails, so it cannot linger", () => {
  const v = classifyPin({
    ...base,
    actual: "v1.2.3",
    exemption: { id: "tinyx", expect: "v1.2.3-7-gabc1234", reason: "because" },
  });
  assert.equal(v.ok, false);
  assert.equal(v.kind, "stale-exemption");
});

// ── Coverage: the half that survives the next module ──────────────────────────

test("a record the pin map has never heard of fails", () => {
  const f = checkPinMapCoverage(["tinymemory", "tinyfuture"], ["tinymemory"]);
  assert.equal(f.length, 1);
  assert.match(f[0], /tinyfuture/);
  assert.match(f[0], /must not inherit an unchecked pin/);
});

test("a pin-map entry for a removed record fails", () => {
  const f = checkPinMapCoverage(["tinymemory"], ["tinymemory", "tinygone"]);
  assert.equal(f.length, 1);
  assert.match(f[0], /tinygone/);
});

test("a fully covered set is clean", () => {
  assert.deepEqual(checkPinMapCoverage(["a", "b"], ["b", "a"]), []);
});

// ── Gitlink parsing and the rewind marker ────────────────────────────────────

test("reads gitlinks out of ls-tree and ignores ordinary blobs", () => {
  const out = [
    "100644 blob 1111111111111111111111111111111111111111\tCargo.toml",
    "160000 commit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tvendor/tinyagents",
    "160000 commit bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\tvendor/tinymemory",
  ].join("\n");
  const m = parseGitlinks(out);
  assert.equal(m.size, 2);
  assert.equal(m.get("vendor/tinyagents"), "a".repeat(40));
});

test("the rewind marker is recognised anywhere on the branch, case-insensitively", () => {
  assert.equal(rewindDeclared("Revert the bump [pin-rewind] because X"), true);
  assert.equal(rewindDeclared("[PIN-REWIND]"), true);
  assert.equal(rewindDeclared("an ordinary bump"), false);
  assert.equal(rewindDeclared(undefined), false);
});

// ── End to end, against the real tree ─────────────────────────────────────────

test(
  "the pin gate passes on the tree as committed",
  { skip: submodulesPresent() ? false : "submodules not checked out" },
  () => {
    const r = run(PINS_CLI);
    assert.equal(r.status, 0, `expected pass, got:\n${r.stdout}${r.stderr}`);
  },
);

test("the monotonicity gate catches 14a23b994, which really did drop a shipped fix", () => {
  // vendor/tinyagents e0f3210 (tinyagents#122) -> bbcd0a6 (#121). Restored by #5796.
  const r = run(MONO_CLI, ["14a23b994^", "14a23b994"], { PR_TITLE: "" });
  if (/cannot resolve base ref|not in the submodule/.test(r.stderr)) {
    // Shallow clone or absent submodule: the gate correctly refused to answer.
    assert.equal(
      r.status,
      1,
      "an unverifiable pin must still fail, never pass",
    );
    return;
  }
  assert.equal(r.status, 1, `expected failure, got:\n${r.stdout}${r.stderr}`);
  assert.match(r.stderr, /vendor\/tinyagents moved BACKWARDS/);
});

test("a rewind declared with [pin-rewind] is allowed through", () => {
  const r = run(MONO_CLI, ["14a23b994^", "14a23b994"], {
    PR_TITLE: "undo the bad bump [pin-rewind]",
  });
  if (/cannot resolve base ref|not in the submodule/.test(r.stderr)) return;
  assert.equal(r.status, 0, `expected pass, got:\n${r.stdout}${r.stderr}`);
});

test("an unresolvable base fails rather than passing on an empty comparison", () => {
  const r = run(MONO_CLI, ["origin/definitely-not-a-ref", "HEAD"]);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /Refusing to pass having compared\s+nothing/);
});

// ── Fail-closed: a check that cannot complete must never report clean ─────────
//
// Two CI gates have shipped here that swallowed a git error and passed having
// scanned nothing (the coverage gate and the IP gate, both via a dead `git` in
// the container). These cases assert the opposite behaviour directly: every
// broken input below must exit NON-ZERO.

/** A throwaway repo root the pins CLI can be pointed at via argv[2]. */
function fixtureRoot(files) {
  const root = mkdtempSync(join(tmpdir(), "module-pins-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(root, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return root;
}

const MINIMAL_WORKFLOW =
  'jobs:\n  x:\n    steps:\n      - run: |\n          memory_version="9.9.9"\n          memory_sha256="' +
  "f".repeat(64) +
  '"\n';

test("an unparseable registry fails the gate instead of passing", () => {
  const root = fixtureRoot({
    "src/openhuman/modules/registry.rs": "// everything here got deleted\n",
    "src/openhuman/modules/memory.rs":
      'pub(crate) const ARTIFACT_CAPABILITIES_PIN: &str = "9.9.9";\n',
    ".github/workflows/ci-full.yml": MINIMAL_WORKFLOW,
    ".github/workflows/ci-lite.yml": MINIMAL_WORKFLOW,
    ".github/workflows/e2e-reusable.yml": MINIMAL_WORKFLOW,
  });
  try {
    const r = run(PINS_CLI, [root]);
    assert.notEqual(
      r.status,
      0,
      `a registry it cannot parse must FAIL, got exit 0:\n${r.stdout}`,
    );
    assert.match(`${r.stdout}${r.stderr}`, /could not find|FAILED/);
    assert.doesNotMatch(
      r.stdout,
      /Module pin check OK/,
      "must not claim OK on an unparseable registry",
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a missing registry file fails the gate instead of passing", () => {
  const root = fixtureRoot({
    ".github/workflows/ci-lite.yml": MINIMAL_WORKFLOW,
  });
  try {
    const r = run(PINS_CLI, [root]);
    assert.notEqual(r.status, 0, "a missing registry must FAIL");
    assert.match(`${r.stdout}${r.stderr}`, /missing|FAILED/);
    assert.doesNotMatch(r.stdout, /Module pin check OK/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("records with no checked-out submodule fail the gate instead of being skipped", () => {
  // A well-formed registry whose vendor/ tree simply is not there — exactly the
  // shape of a lane that forgot `submodules: recursive`. Skipping would report a
  // clean scan of nothing.
  const registry = [
    "const TINYDOCS: ModuleRecord = ModuleRecord {",
    '    id: "tinydocs",',
    '    version: "0.1.14",',
    "    assets: &[],",
    "};",
    "",
    "pub const ALL: &[ModuleRecord] = &[",
    "    TINYDOCS,",
    "];",
    "",
  ].join("\n");
  const root = fixtureRoot({
    "src/openhuman/modules/registry.rs": registry,
    "src/openhuman/modules/memory.rs":
      'pub(crate) const ARTIFACT_CAPABILITIES_PIN: &str = "1.12.0";\n',
    ".github/workflows/ci-full.yml": MINIMAL_WORKFLOW,
    ".github/workflows/ci-lite.yml": MINIMAL_WORKFLOW,
    ".github/workflows/e2e-reusable.yml": MINIMAL_WORKFLOW,
  });
  try {
    const r = run(PINS_CLI, [root]);
    assert.notEqual(
      r.status,
      0,
      "an uninitialised submodule must FAIL, not skip",
    );
    assert.match(
      `${r.stdout}${r.stderr}`,
      /not a checked-out submodule|submodules: recursive/,
    );
    assert.doesNotMatch(r.stdout, /Module pin check OK/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the monotonicity gate fails when run outside a git repo rather than passing", () => {
  const root = mkdtempSync(join(tmpdir(), "module-pins-nogit-"));
  try {
    const r = spawnSync(process.execPath, [MONO_CLI, "origin/main", "HEAD"], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env },
    });
    assert.notEqual(r.status, 0, "no git repo must FAIL, not pass");
    assert.doesNotMatch(r.stdout, /check OK/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// ── Review findings from #5812 ────────────────────────────────────────────────

test("a comment inside ALL does not swallow the record beneath it", () => {
  // CodeRabbit on #5812: splitting on commas before stripping comments puts the
  // comment and the next record in one token, and filtering tokens beginning
  // with `//` dropped both. A dropped record reads downstream as "missing from
  // PIN_MAP" — CI fails while pointing at the wrong thing.
  const src = [
    "pub const ALL: &[ModuleRecord] = &[",
    "    // the codecs",
    "    TINYDOCS,",
    "    TINYWALLET, // lazy",
    "    // Eager, unlike the two codecs above.",
    "    TINYMEMORY,",
    "];",
  ].join("\n");
  assert.deepEqual(parseAllList(src), ["TINYDOCS", "TINYWALLET", "TINYMEMORY"]);
});

test("a divergent pin is not forward", () => {
  // Codex on #5812: asking only "is head an ancestor of base?" reads the single
  // failed query as forward, so siblings — a side branch, or a rebased submodule
  // history — sail through. Divergence loses the base's commits exactly as a
  // rewind does, and a rewind onto a sibling is one rebase away from 14a23b994.
  assert.equal(
    classifyMove({ headIsAncestorOfBase: true, baseIsAncestorOfHead: false }),
    "rewind",
  );
  assert.equal(
    classifyMove({ headIsAncestorOfBase: false, baseIsAncestorOfHead: true }),
    "forward",
  );
  assert.equal(
    classifyMove({ headIsAncestorOfBase: false, baseIsAncestorOfHead: false }),
    "divergent",
  );
});

test("the submodule probe is not fooled by the superproject above it", () => {
  // Codex on #5812. `rev-parse --git-dir` walks upward and succeeds from any
  // directory inside a repository, submodule or not — so it cannot answer this.
  const root = mkdtempSync(join(tmpdir(), "module-pins-super-"));
  try {
    execFileSync("git", ["init", "-q", root], { stdio: "ignore" });
    const notASubmodule = join(root, "vendor", "tinymemory");
    mkdirSync(notASubmodule, { recursive: true });

    // The old probe: succeeds, answering the SUPERPROJECT's git dir.
    const walked = execFileSync(
      "git",
      ["-C", notASubmodule, "rev-parse", "--git-dir"],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    ).trim();
    assert.ok(
      walked.length > 0,
      "precondition: --git-dir does walk up, which is why it cannot be used",
    );

    // The probe actually used: the toplevel is the superproject, not the path,
    // so this correctly reports "not a checked-out submodule".
    const top = execFileSync(
      "git",
      ["-C", notASubmodule, "rev-parse", "--show-toplevel"],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    ).trim();
    assert.equal(
      toplevelProvesSubmodule(realpathSync(top), realpathSync(notASubmodule)),
      false,
      "a directory that merely sits inside a repo must not read as a checked-out submodule",
    );

    // ...and the positive case: the superproject root IS its own toplevel.
    const superTop = execFileSync(
      "git",
      ["-C", root, "rev-parse", "--show-toplevel"],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    ).trim();
    assert.equal(
      toplevelProvesSubmodule(realpathSync(superTop), realpathSync(root)),
      true,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
