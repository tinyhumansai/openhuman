// Tests for scripts/modules/release-and-repin.mjs (tinyhumansai/openhuman#7615).
// Fixtures only: gh and git are faked, no release is dispatched.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  bumpVersion,
  dropExemption,
  parseChecksumToml,
  rewriteRecord,
  unusedChecksums,
  updateProviderPins,
} from "../lib/module-repin.mjs";
import { parseArgs, repin } from "../modules/release-and-repin.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const OLD = "a".repeat(64);
const SHA_X = "1".repeat(64);
const SHA_A = "2".repeat(64);
const CHECKSUM = `[sha256]
"tinyx-module-0.3.0-ubuntu-24.04-x86_64.tar.gz" = "${SHA_X}"
"tinyx-module-0.3.0-macos-15-arm64.tar.gz" = "${SHA_A}"
"tinyx-module-0.3.0-windows-2025-x86_64.zip" = "${"3".repeat(64)}"
`;

const RECORDS = `
pub(crate) const OTHER: ModuleRecord = ModuleRecord {
    id: "other",
    version: "0.2.0",
    release_url: "https://github.com/tinyhumansai/other/releases/tag/v0.2.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "other-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "${OLD}",
        },
    ],
    source: SourceSpec {
        submodule: "other",
        manifest: "Cargo.toml",
        package: "other-module",
        lib_name: "other_module",
        features: &[],
    },
};

pub(crate) const TINYX: ModuleRecord = ModuleRecord {
    id: "tinyx",
    version: "0.2.0",
    release_url: "https://github.com/tinyhumansai/tinyx/releases/tag/v0.2.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyx-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "${OLD}",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyx-module-0.2.0-macos-15-arm64.tar.gz",
            sha256: "${OLD}",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyx-module-0.2.0-windows-2025-x86_64.zip",
            sha256: "${OLD}",
        },
    ],
    source: SourceSpec {
        submodule: "tinyx",
        manifest: "Cargo.toml",
        package: "tinyx-module",
        lib_name: "tinyx_module",
        features: &[],
    },
};
`;

test("parseChecksumToml reads the [sha256] table and rejects bad digests", () => {
  assert.equal(parseChecksumToml(CHECKSUM).size, 3);
  assert.throws(() => parseChecksumToml('[sha256]\n"a.tar.gz" = "xyz"\n'), /64-hex/);
  assert.throws(() => parseChecksumToml("[other]\n"), /no \[sha256\]/);
});

test("rewriteRecord takes version, url, archives and digests verbatim from checksum.toml", () => {
  const sums = parseChecksumToml(CHECKSUM);
  const { src, changed } = rewriteRecord(RECORDS, "tinyx", "0.3.0", sums);
  assert.ok(changed);
  assert.match(src, /id: "tinyx",\s+version: "0\.3\.0"/);
  assert.match(src, /tinyx\/releases\/tag\/v0\.3\.0"/);
  assert.match(src, new RegExp(`tinyx-module-0\\.3\\.0-ubuntu-24\\.04-x86_64\\.tar\\.gz",\\s+sha256: "${SHA_X}"`));
  assert.match(src, new RegExp(`macos-15-arm64\\.tar\\.gz",\\s+sha256: "${SHA_A}"`));
  assert.match(src, /windows-2025-x86_64\.zip",\s+sha256: "3{64}"/);
  // The neighbouring record is untouched.
  assert.match(src, /other-module-0\.2\.0-ubuntu-24\.04-x86_64\.tar\.gz",\s+sha256: "a{64}"/);
  assert.equal(src.match(/version: "0\.2\.0"/g).length, 1);
  assert.deepEqual(unusedChecksums(src, "tinyx", sums), []);
});

test("rewriteRecord uses the selected fork release URL when provided", () => {
  const releaseUrl = "https://github.com/senamakel/tinyx/releases/tag/v0.3.0";
  const { src } = rewriteRecord(
    RECORDS,
    "tinyx",
    "0.3.0",
    parseChecksumToml(CHECKSUM),
    releaseUrl,
  );

  assert.match(src, new RegExp(`release_url: "${releaseUrl}"`));
});

test("rewriteRecord refuses a release that lacks an asset, and an unknown id is unchanged", () => {
  const partial = parseChecksumToml(
    `[sha256]\n"tinyx-module-0.3.0-ubuntu-24.04-x86_64.tar.gz" = "${SHA_X}"\n`,
  );
  assert.throws(() => rewriteRecord(RECORDS, "tinyx", "0.3.0", partial), /no entry for/);
  const none = rewriteRecord(RECORDS, "nope", "0.3.0", parseChecksumToml(CHECKSUM));
  assert.equal(none.changed, false);
  assert.equal(none.src, RECORDS);
});

test("the helper never computes a digest", () => {
  for (const file of ["../lib/module-repin.mjs", "../modules/release-and-repin.mjs"]) {
    const text = readFileSync(resolve(HERE, file), "utf8");
    assert.doesNotMatch(text, /node:crypto|createHash|sha256sum|shasum|openssl/, file);
  }
});

test("bumpVersion and parseArgs", () => {
  assert.equal(bumpVersion("0.2.9", "patch"), "0.2.10");
  assert.equal(bumpVersion("0.2.9", "minor"), "0.3.0");
  assert.equal(bumpVersion("0.2.9", "major"), "1.0.0");
  assert.throws(() => bumpVersion("0.2.9", "huge"), /--bump/);
  assert.deepEqual(parseArgs(["tinyx", "--bump", "minor", "--yes"]), {
    bump: "minor",
    yes: true,
    dryRun: false,
    owner: "tinyhumansai",
    workflowInput: "bump",
    id: "tinyx",
  });
  assert.throws(() => parseArgs([]), /usage/);
  assert.throws(() => parseArgs(["x", "--bump", "no"]), /--bump/);
});

test("provider pins and exemptions are updated only for the named id", () => {
  const pins = { providers: { tinyx: { repo: "o/r", version: "0.2.0", built_against: "b" }, y: { version: "1" } } };
  const out = updateProviderPins(pins, "tinyx", "0.3.0", "c".repeat(40));
  assert.equal(out.json.providers.tinyx.version, "0.3.0");
  assert.equal(out.json.providers.tinyx.built_against, "c".repeat(40));
  assert.equal(out.json.providers.y.version, "1");
  assert.equal(updateProviderPins(pins, "zzz", "1.0.0", "c").changed, false);
  const ex = dropExemption({ exemptions: [{ id: "tinyx" }, { id: "y" }] }, "tinyx");
  assert.deepEqual(ex.json.exemptions, [{ id: "y" }]);
});

// ── the whole flow, with gh and git faked ────────────────────────────────────

function tree() {
  const root = mkdtempSync(join(tmpdir(), "repin-fixture-"));
  const dir = join(root, "crates/openhuman-core/src/modules/registry");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "records_a.rs"), RECORDS);
  mkdirSync(join(root, "scripts/ci"), { recursive: true });
  writeFileSync(join(root, "scripts/ci/module-pin-exemptions.json"), JSON.stringify({ exemptions: [{ id: "tinyx", expect: "v0", reason: "r" }] }));
  writeFileSync(join(root, "scripts/ci/module-provider-pins.json"), JSON.stringify({ providers: {} }));
  return root;
}

function fakeRun(calls, { checksum = CHECKSUM } = {}) {
  return (cmd, args, opts = {}) => {
    calls.push([cmd, ...args]);
    if (cmd === "gh" && args[0] === "release" && args[1] === "download") {
      writeFileSync(join(args[args.indexOf("-D") + 1], "checksum.toml"), checksum);
    }
    if (cmd === "gh" && args[0] === "run" && args[1] === "list") return { code: 0, stdout: "42\n", stderr: "" };
    if (cmd === "gh" && args[0] === "release" && args[1] === "view") return { code: 0, stdout: "v0.3.0\n", stderr: "" };
    if (cmd === "git" && args[0] === "rev-parse") return { code: 0, stdout: "d".repeat(40) + "\n", stderr: "" };
    void opts;
    return { code: 0, stdout: "", stderr: "" };
  };
}

test("an existing release is re-pinned without dispatching anything", async () => {
  const root = tree();
  const calls = [];
  try {
    const res = await repin({ id: "tinyx", version: "0.3.0", bump: "patch", root }, { run: fakeRun(calls), confirm: async () => assert.fail("must not ask"), log: () => {} });
    assert.equal(res.version, "0.3.0");
    assert.deepEqual(res.failed, []);
    assert.ok(!calls.some((c) => c.includes("workflow")), "no dispatch");
    assert.ok(calls.some((c) => c[0] === "git" && c[1] === "checkout" && c[2] === "v0.3.0"));
    assert.ok(calls.some((c) => c[0] === "git" && c[1] === "add" && c[2] === "vendor/tinyx"));
    assert.ok(calls.some((c) => c[0] === "node" && c[1].endsWith("check-module-pins.mjs")));
    assert.ok(calls.some((c) => c[0] === "node" && c[1].endsWith("check-submodule-monotonic.mjs")));
    const src = readFileSync(join(root, "crates/openhuman-core/src/modules/registry/records_a.rs"), "utf8");
    assert.match(src, new RegExp(SHA_X));
    assert.deepEqual(JSON.parse(readFileSync(join(root, "scripts/ci/module-pin-exemptions.json"), "utf8")).exemptions, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("dispatching a release asks first and a refusal dispatches nothing", async () => {
  const root = tree();
  const calls = [];
  try {
    await assert.rejects(
      repin({ id: "tinyx", bump: "minor", root }, { run: fakeRun(calls), confirm: async () => false, log: () => {} }),
      /not confirmed/,
    );
    assert.deepEqual(calls, []);

    const asked = [];
    const res = await repin({ id: "tinyx", bump: "minor", root }, {
      run: fakeRun(calls),
      confirm: async (q) => (asked.push(q), true),
      log: () => {},
    });
    assert.equal(asked.length, 1);
    assert.match(asked[0], /hard to undo/);
    assert.equal(res.version, "0.3.0");
    const dispatch = calls.find((c) => c[1] === "workflow");
    assert.deepEqual(dispatch, ["gh", "workflow", "run", "release.yml", "-R", "tinyhumansai/tinyx", "-f", "bump=minor"]);
    assert.ok(calls.findIndex((c) => c[1] === "workflow") < calls.findIndex((c) => c[1] === "release" && c[2] === "download"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("fork release dispatch targets the selected owner and ref", async () => {
  const root = tree();
  const calls = [];
  const ref = "epic/8131-storage-primitives";
  try {
    const res = await repin(
      {
        id: "tinyx",
        owner: "senamakel",
        ref,
        workflowInput: "tag",
        version: "0.3.0",
        root,
      },
      { run: fakeRun(calls), confirm: async () => true, log: () => {} },
    );
    assert.equal(res.version, "0.3.0");
    assert.deepEqual(res.failed, []);
    assert.ok(
      calls.some((call) =>
        JSON.stringify(call) ===
        JSON.stringify([
          "gh",
          "workflow",
          "run",
          "release.yml",
          "-R",
          "senamakel/tinyx",
          "--ref",
          ref,
          "-f",
          "tag=v0.3.0",
        ]),
      ),
    );
    assert.ok(
      calls.some((call) =>
        call[0] === "gh" &&
        call[1] === "run" &&
        call.includes("--branch") &&
        call.includes(ref),
      ),
      "the helper watches the dispatched fork branch rather than the fork default branch",
    );
    assert.ok(
      calls.some((call) =>
        call[0] === "gh" &&
        call[1] === "release" &&
        call[2] === "download" &&
        call.includes("senamakel/tinyx"),
      ),
      "checksums come from the selected fork repository",
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("--dry-run changes and dispatches nothing", async () => {
  const root = tree();
  const calls = [];
  try {
    const before = readFileSync(join(root, "crates/openhuman-core/src/modules/registry/records_a.rs"), "utf8");
    const res = await repin({ id: "tinyx", bump: "patch", dryRun: true, root }, { run: fakeRun(calls), log: () => {} });
    assert.ok(res.dryRun);
    assert.deepEqual(calls, []);
    assert.equal(readFileSync(join(root, "crates/openhuman-core/src/modules/registry/records_a.rs"), "utf8"), before);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a failed pin gate is reported as failed", async () => {
  const root = tree();
  const calls = [];
  const run = fakeRun(calls);
  try {
    const res = await repin({ id: "tinyx", version: "0.3.0", bump: "patch", root }, {
      run: (cmd, args, o) => (cmd === "node" && args[0].endsWith("check-module-pins.mjs") ? { code: 1, stdout: "", stderr: "drift" } : run(cmd, args, o)),
      log: () => {},
    });
    assert.deepEqual(res.failed, ["check-module-pins.mjs"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
