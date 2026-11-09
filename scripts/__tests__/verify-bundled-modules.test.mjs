// Co-located tests for scripts/release/verify-bundled-modules.mjs — the gate
// that catches pin drift between the staged bundled-modules tree and the
// sha256 compiled into the Rust module registry (openhuman#8500).
//
// Each case injects a fake registry source so the test does not depend on the
// real `crates/openhuman-core/src/modules/registry/*.rs` fragments — which
// change release-over-release — and a scratch directory stands in for a
// staged `bundled-modules/` tree.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { verifyBundledModules } from "../release/verify-bundled-modules.mjs";

const sha256Hex = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");

/**
 * Build the minimum registry source that `parseAllList` + `parseRecords`
 * (scripts/lib/module-pins.mjs) accept: one `ModuleRecord` with a Linux
 * archive asset and a macOS archive asset, both pinned to the given sha256s.
 */
function makeSource({
  linuxSha,
  macSha,
  id = "tinymock",
  version = "0.1.0",
} = {}) {
  const archiveLinux = `${id}-${version}-ubuntu-22.04-x86_64.tar.gz`;
  const archiveMac = `${id}-${version}-macos-15-arm64.tar.gz`;
  return `
const TINYMOCK: ModuleRecord = ModuleRecord {
    id: "${id}",
    version: "${version}",
    release_url: "https://example.invalid/${id}/releases/tag/v${version}",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "${archiveLinux}",
            sha256: "${linuxSha}",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "${archiveMac}",
            sha256: "${macSha}",
            library_sha256: None,
        },
    ],
};
pub const ALL: &[ModuleRecord] = &[TINYMOCK];
`;
}

const makeDir = () => mkdtempSync(join(tmpdir(), "verify-bundled-"));

function writeArchive(root, { id, version, hostKey, archive, bytes }) {
  const dir = join(root, id, version, hostKey);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, archive), bytes);
}

function writeMarker(root, { id, version, hostKey, archive, content }) {
  const dir = join(root, id, version, hostKey);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${archive}.sha256`), content);
}

test("happy path: archive sha matches the registry pin", () => {
  const dir = makeDir();
  const bytes = Buffer.from("hello openhuman");
  const linuxSha = sha256Hex(bytes);
  const source = makeSource({ linuxSha, macSha: "0".repeat(64) });
  writeArchive(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "ubuntu-22.04-x86_64",
    archive: "tinymock-0.1.0-ubuntu-22.04-x86_64.tar.gz",
    bytes,
  });

  const result = verifyBundledModules({ dir, source });

  assert.equal(result.checked, 1);
  assert.deepEqual(result.problems, []);
});

test("mismatch: staged archive bytes drift from the pin", () => {
  const dir = makeDir();
  const bytes = Buffer.from("tampered");
  const pin = "a".repeat(64); // the pin the registry claims
  const source = makeSource({ linuxSha: pin, macSha: "0".repeat(64) });
  writeArchive(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "ubuntu-22.04-x86_64",
    archive: "tinymock-0.1.0-ubuntu-22.04-x86_64.tar.gz",
    bytes,
  });

  const { checked, problems } = verifyBundledModules({ dir, source });

  assert.equal(checked, 1);
  assert.equal(problems.length, 1);
  const [problem] = problems;
  assert.equal(problem.kind, "mismatch");
  assert.equal(problem.expected, pin);
  assert.equal(problem.actual, sha256Hex(bytes));
  // The message cites the exact Sentry string and the issue number so a CI
  // failure reads the same way the Sentry alert does.
  assert.match(
    problem.message,
    /release is not cached and downloads are disabled/,
  );
  assert.match(problem.message, /TAURI-RUST-11MZ/);
  assert.match(problem.message, /#8500/);
});

test("unknown: staged (id, version, hostKey, archive) is not in the pins", () => {
  const dir = makeDir();
  const source = makeSource({
    linuxSha: "a".repeat(64),
    macSha: "0".repeat(64),
  });
  writeArchive(dir, {
    id: "unknownmod",
    version: "9.9.9",
    hostKey: "ubuntu-22.04-x86_64",
    archive: "unknownmod-9.9.9-ubuntu-22.04-x86_64.tar.gz",
    bytes: Buffer.from("x"),
  });

  const { checked, problems } = verifyBundledModules({ dir, source });

  assert.equal(checked, 1);
  assert.equal(problems.length, 1);
  assert.equal(problems[0].kind, "unknown");
  assert.match(
    problems[0].message,
    /staged file has no matching registry pin/,
  );
});

test("macOS marker: content matches the pin → passes", () => {
  const dir = makeDir();
  const macSha = "b".repeat(64);
  const source = makeSource({ linuxSha: "a".repeat(64), macSha });
  writeMarker(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "macos-15-arm64",
    archive: "tinymock-0.1.0-macos-15-arm64.tar.gz",
    content: `${macSha}\n`,
  });

  const { checked, problems } = verifyBundledModules({ dir, source });

  assert.equal(checked, 1);
  assert.deepEqual(problems, []);
});

test("macOS marker: content drifts from the pin → mismatch", () => {
  const dir = makeDir();
  const macSha = "b".repeat(64);
  const stale = "c".repeat(64);
  const source = makeSource({ linuxSha: "a".repeat(64), macSha });
  writeMarker(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "macos-15-arm64",
    archive: "tinymock-0.1.0-macos-15-arm64.tar.gz",
    content: `${stale}\n`,
  });

  const { checked, problems } = verifyBundledModules({ dir, source });

  assert.equal(checked, 1);
  assert.equal(problems.length, 1);
  assert.equal(problems[0].kind, "mismatch");
  assert.equal(problems[0].expected, macSha);
  assert.equal(problems[0].actual, stale);
});

test("wrong-type: a sha256 marker on a Linux host does not satisfy its archive pin", () => {
  // walkStaged strips `.sha256` from a marker, so an archive and a marker on
  // the same hostKey produce an identical key. Before the type gate, a
  // marker whose content equals the archive pin would pass pin verification
  // AND clear the completeness check on a Linux build. Guard against that.
  const dir = makeDir();
  const linuxArchiveSha = "a".repeat(64);
  const source = makeSource({
    linuxSha: linuxArchiveSha,
    macSha: "0".repeat(64),
  });
  writeMarker(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "ubuntu-22.04-x86_64",
    archive: "tinymock-0.1.0-ubuntu-22.04-x86_64.tar.gz",
    // Marker content equals the archive pin — pre-fix this passed.
    content: `${linuxArchiveSha}\n`,
  });

  const { checked, problems } = verifyBundledModules({
    dir,
    source,
    hostKey: "ubuntu-22.04-x86_64",
  });

  assert.equal(checked, 1);
  // Two problems: wrong-type on the marker, and missing for the archive the
  // staged marker's key was NOT allowed to clear from the completeness set.
  const wrongType = problems.find((p) => p.kind === "wrong-type");
  const missing = problems.find((p) => p.kind === "missing");
  assert.ok(wrongType, "expected wrong-type problem");
  assert.equal(wrongType.entry.kind, "marker");
  assert.match(wrongType.message, /required archive/);
  assert.ok(missing, "wrong-type must not clear the archive from mustStage");
});

test("target completeness: a staged tree missing an expected host asset reports it", () => {
  const dir = makeDir();
  const linuxBytes = Buffer.from("linux ok");
  const linuxSha = sha256Hex(linuxBytes);
  const source = makeSource({
    linuxSha,
    macSha: "b".repeat(64), // the mac asset the registry expects, left unstaged
  });
  // Stage only the Linux archive. The macOS asset is in the pins for this
  // host-level build matrix cell, but absent from disk — a partial stage.
  writeArchive(dir, {
    id: "tinymock",
    version: "0.1.0",
    hostKey: "ubuntu-22.04-x86_64",
    archive: "tinymock-0.1.0-ubuntu-22.04-x86_64.tar.gz",
    bytes: linuxBytes,
  });

  // Without a hostKey: the Linux archive passes and the macOS gap is invisible.
  const open = verifyBundledModules({ dir, source });
  assert.equal(open.checked, 1);
  assert.deepEqual(open.problems, []);

  // With hostKey="macos-15-arm64": the completeness check fires. The staged
  // Linux archive is still inspected (and found correct) but it does not
  // belong to this host, so it is also flagged as unknown-for-this-host.
  const strict = verifyBundledModules({
    dir,
    source,
    hostKey: "macos-15-arm64",
  });
  const missing = strict.problems.find((p) => p.kind === "missing");
  assert.ok(missing, "expected a missing-asset problem");
  assert.equal(missing.hostKey, "macos-15-arm64");
  assert.match(missing.key, /^tinymock\/0\.1\.0\/macos-15-arm64\//);
  assert.match(missing.message, /partial stage/);
});

test("empty tree: throws explaining stage-modules.mjs was not run", () => {
  const dir = makeDir();
  const source = makeSource({
    linuxSha: "a".repeat(64),
    macSha: "0".repeat(64),
  });

  assert.throws(
    () => verifyBundledModules({ dir, source }),
    /no archives or markers under .*; was `stage-modules\.mjs` run\?/,
  );
});
