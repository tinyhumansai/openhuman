// Tests for scripts/modules/pin-status.mjs (tinyhumansai/openhuman#7615):
// in sync, ahead, behind, diverged and exemption cases, then the CLI against a
// fixture tree built from throwaway git repositories.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  aheadIds,
  AHEAD_MESSAGE,
  classifyStatus,
  parseDescribe,
} from "../lib/module-pin-status.mjs";
import { main } from "../modules/pin-status.mjs";

const base = { id: "tinyx", version: "0.2.0", submodulePath: "vendor/tinyx" };
const kind = (over) => classifyStatus({ ...base, ...over }).kind;

test("parseDescribe reads exact tags and N-past forms", () => {
  assert.deepEqual(parseDescribe("v1.2.3"), {
    tag: "v1.2.3",
    parts: [1, 2, 3],
    commitsPast: 0,
  });
  assert.equal(parseDescribe("v1.2.3-7-gabcdef01").commitsPast, 7);
  assert.equal(parseDescribe("nonsense"), null);
});

test("a gitlink on the registry tag is in sync", () => {
  assert.equal(kind({ actual: "v0.2.0", wantedIsAncestor: true }), "in-sync");
});

test("a gitlink commits past the pinned tag is ahead", () => {
  const s = classifyStatus({ ...base, actual: "v0.2.0-3-gabcdef01", wantedIsAncestor: true });
  assert.equal(s.kind, "ahead");
  assert.match(s.message, /you're running a locally compiled module/);
  assert.ok(s.message.startsWith(AHEAD_MESSAGE));
});

test("a gitlink on a newer release than the registry is ahead", () => {
  assert.equal(kind({ actual: "v0.2.2", wantedIsAncestor: true }), "ahead");
});

test("a gitlink on an older release than the registry is behind", () => {
  const s = classifyStatus({ ...base, actual: "v0.1.9", wantedIsAncestor: true });
  assert.equal(s.kind, "behind");
  assert.match(s.message, /behind\/diverged/);
});

test("a registry tag that is not an ancestor is diverged", () => {
  assert.equal(kind({ actual: "v0.2.2", wantedIsAncestor: false }), "diverged");
  assert.equal(kind({ actual: "garbage" }), "diverged");
});

test("a declared exemption reports exempt only while the drift is unchanged", () => {
  const exemption = { id: "tinyx", expect: "v0.1.17-159-gf95c461e", reason: "known" };
  assert.equal(kind({ actual: "v0.1.17-159-gf95c461e", exemption }), "exempt");
  // Drift moved: falls back to a real classification, and says why.
  const moved = classifyStatus({ ...base, actual: "v0.1.17-200-g00000000", exemption, wantedIsAncestor: true });
  assert.equal(moved.kind, "behind");
  assert.match(moved.message, /moved past its module-pin-exemptions\.json entry/);
  // Pins agree but the exemption lingers: in sync, flagged.
  const stale = classifyStatus({ ...base, actual: "v0.2.0", exemption });
  assert.equal(stale.kind, "in-sync");
  assert.match(stale.message, /stale/);
});

test("aheadIds lists only ahead modules", () => {
  assert.deepEqual(
    aheadIds([{ id: "a", kind: "ahead" }, { id: "b", kind: "in-sync" }, { id: "c", kind: "behind" }]),
    ["a"],
  );
});

// ── CLI against a fixture tree ───────────────────────────────────────────────

const git = (cwd, ...args) =>
  execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", ...args], {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();

function record(name, id, version, sub) {
  return `
pub(crate) const ${name}: ModuleRecord = ModuleRecord {
    id: "${id}",
    version: "${version}",
    release_url: "https://github.com/tinyhumansai/${sub}/releases/tag/v${version}",
    assets: &[],
    source: SourceSpec {
        submodule: "${sub}",
        manifest: "Cargo.toml",
        package: "${sub}-module",
        lib_name: "${sub}_module",
        features: &[],
    },
};
`;
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "pin-status-"));
  const reg = join(root, "crates/openhuman-core/src/modules");
  mkdirSync(join(reg, "registry"), { recursive: true });
  writeFileSync(join(reg, "registry.rs"), "");
  writeFileSync(
    join(reg, "registry/records_a.rs"),
    record("SYNCED", "synced", "1.0.0", "synced") +
      record("AHEAD", "ahead", "1.0.0", "ahead") +
      record("MISSING", "missing", "1.0.0", "missing"),
  );
  mkdirSync(join(root, "scripts/ci"), { recursive: true });
  writeFileSync(join(root, "scripts/ci/module-pin-exemptions.json"), '{"exemptions":[]}');
  for (const sub of ["synced", "ahead"]) {
    const dir = join(root, "vendor", sub);
    mkdirSync(dir, { recursive: true });
    git(dir, "init", "-q");
    writeFileSync(join(dir, "f"), "1");
    git(dir, "add", ".");
    git(dir, "commit", "-qm", "one");
    git(dir, "tag", "v1.0.0");
    if (sub === "ahead") {
      writeFileSync(join(dir, "f"), "2");
      git(dir, "commit", "-qam", "two");
    }
  }
  return root;
}

test("the CLI reports each fixture module and exports the ahead ids", () => {
  const root = fixture();
  try {
    const { lines, code } = main(["--print-env"], root);
    const text = lines.join("\n");
    assert.match(text, /synced\s+in-sync/);
    assert.match(text, /ahead\s+ahead\s+gitlink ahead of released pin/);
    assert.match(text, /missing\s+unavailable\s+vendor\/missing is not checked out/);
    assert.match(text, /OPENHUMAN_MODULE_PIN_AHEAD=ahead$/);
    assert.equal(code, 0, "advisory unless --strict");
    assert.equal(main(["--strict"], root).code, 1);
    const hint = main(["--hint"], root).lines.join("\n");
    assert.doesNotMatch(hint, /synced/);
    assert.match(hint, /release-and-repin\.mjs/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
