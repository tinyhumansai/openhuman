// Tests for the module-pin gates (openhuman#5727).
//
// The pure logic is covered directly; the two CLIs are driven end-to-end against
// the real tree, including the two commits that actually shipped a regression.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  unlinkSync,
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
  classifyProviderPin,
  expandRustIncludes,
  parseAllList,
  parseGitlinks,
  parseRecords,
  rewindDeclared,
  toplevelProvesSubmodule,
  validateTestOnlySourceParity,
} from "../lib/module-pins.mjs";
import { classifyStatus } from "../lib/module-pin-status.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const PINS_CLI = join(REPO_ROOT, "scripts/ci/check-module-pins.mjs");
const MONO_CLI = join(REPO_ROOT, "scripts/ci/check-submodule-monotonic.mjs");
const readRustModule = (relativePath) =>
  expandRustIncludes(relativePath, (includedPath) =>
    readFileSync(join(REPO_ROOT, includedPath), "utf8"),
  );
const readRegistry = () => {
  const relativePath = "crates/openhuman-core/src/modules/registry.rs";
  const entry = readRustModule(relativePath);
  const recordsDir = relativePath.replace(/\.rs$/, "");
  const recordModules = [
    ...entry.matchAll(/^mod (records_[a-z0-9_]+);$/gm),
  ].map(([, name]) => `${recordsDir}/${name}.rs`);
  return [entry, ...recordModules.map(readRustModule)].join("\n");
};
const run = (cli, args = [], env = {}) =>
  spawnSync(process.execPath, [cli, ...args], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });

/**
 * Are all module sources checked out with enough tag history for the gate?
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
 * the submodule genuinely checked out. The gate also calls `git describe`, so
 * this end-to-end test needs tags to be present. Paths are compared through
 * `realpathSync` because macOS resolves `/tmp/...` to `/private/tmp/...`.
 */
function submodulesPresent() {
  const paths = [
    "vendor/tinydocs",
    "vendor/tinywallet",
    "vendor/tinyjuice",
    "vendor/tinyvoice",
    "vendor/tinymcp",
    "vendor/tinyconnectors",
  ];
  try {
    return paths.every((relativePath) => {
      const path = join(REPO_ROOT, relativePath);
      const top = execFileSync(
        "git",
        ["-C", path, "rev-parse", "--show-toplevel"],
        {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "ignore"],
        },
      ).trim();
      if (!toplevelProvesSubmodule(realpathSync(top), realpathSync(path)))
        return false;
      execFileSync("git", ["-C", path, "describe", "--tags", "HEAD"], {
        stdio: "ignore",
      });
      return true;
    });
  } catch {
    return false;
  }
}

// This reads the release manifest directly, so a stale voice gitlink is caught
// even in a shallow checkout without the tags used by the general pin gate.
test("voice source release matches the artifact used for recording summaries", () => {
  const record = [...parseRecords(readRegistry()).values()].find(
    (record) => record.id === "tinyvoice",
  );
  assert.ok(record, "voice adapter requires a registered module");
  const source = readFileSync(
    join(REPO_ROOT, "vendor/tinyvoice/crates/tinyvoice-module/Cargo.toml"),
    "utf8",
  );
  const version = source.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.ok(version, "voice module source must declare its release version");
  assert.equal(
    version,
    record.version,
    "the host must compile against the same voice release it loads",
  );
});

test("search source release matches the loaded search artifact", () => {
  const record = [...parseRecords(readRegistry()).values()].find(
    (record) => record.id === "tinysearch",
  );
  assert.ok(record, "search adapter requires a registered module");
  const source = readFileSync(
    join(REPO_ROOT, "vendor/tinysearch/Cargo.toml"),
    "utf8",
  );
  const version = source.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.equal(
    version,
    record.version,
    "search source must match its loaded artifact",
  );
});

test("wallet source release matches the loaded wallet artifact", () => {
  const record = [...parseRecords(readRegistry()).values()].find(
    (record) => record.id === "tinywallet",
  );
  assert.ok(record, "wallet adapter requires a registered module");
  const source = readFileSync(
    join(REPO_ROOT, "vendor/tinywallet/Cargo.toml"),
    "utf8",
  );
  const version = source.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.equal(
    version,
    record.version,
    "wallet source must match its loaded artifact",
  );
});

// ── Registry parsing ──────────────────────────────────────────────────────────

test("parses every record `ALL` lists out of the real registry", () => {
  const src = readRegistry();
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
  const src = readRegistry();
  const rec = [...parseRecords(src).values()].find((r) => r.id === "tinyjuice");
  assert.ok(rec, "tinyjuice record not found");
  assert.ok(rec.assets.length > 0, "tinyjuice publishes no assets?");
  for (const a of rec.assets) {
    assert.match(a.sha256, /^[0-9a-f]{64}$/, `bad digest for ${a.hostKey}`);
    assert.ok(
      a.archive.includes(rec.version),
      `${a.archive} does not carry version ${rec.version}`,
    );
  }
});

test("reading a registry with no ALL block throws rather than returning empty", () => {
  assert.throws(() => parseAllList("fn main() {}"), /could not find/);
  assert.throws(() => parseRecords("fn main() {}"), /parsed zero/);
});

test("parses a crate-visible record definition", () => {
  const records = parseRecords(
    'pub(crate) const TINYTEST: ModuleRecord = ModuleRecord {\n  id: "tinytest",\n  version: "1.2.3",\n  assets: &[],\n};',
  );
  assert.equal(records.get("TINYTEST")?.id, "tinytest");
});

test("parses assets with an optional library digest", () => {
  const records = parseRecords(`const TINYTEST: ModuleRecord = ModuleRecord {
    id: "tinytest",
    version: "1.2.3",
    assets: &[
        PlatformAsset {
            host_key: "linux-x86_64",
            archive: "tinytest.tar.gz",
            sha256: "${"a".repeat(64)}",
            library_sha256: None,
        },
    ],
};`);
  assert.deepEqual(records.get("TINYTEST")?.assets, [
    {
      hostKey: "linux-x86_64",
      archive: "tinytest.tar.gz",
      sha256: "a".repeat(64),
    },
  ]);
});

// ── Provider records ──────────────────────────────────────────────────────────

const SHA_A = "a".repeat(40);
const SHA_B = "b".repeat(40);

function sourceParityFixture() {
  const root = mkdtempSync(join(tmpdir(), "module-source-parity-"));
  mkdirSync(root, { recursive: true });
  const git = (args) =>
    execFileSync("git", args, { cwd: root, stdio: "ignore" });
  git(["init", "-q"]);
  git(["config", "user.email", "pin-check@example.test"]);
  git(["config", "user.name", "Pin Check"]);
  git(["config", "commit.gpgsign", "false"]);
  git(["config", "tag.gpgsign", "false"]);
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(
    join(root, "src/lib.rs"),
    '#[cfg(test)]\n#[path = "tests.rs"]\nmod tests;\npub fn value() -> u8 { 1 }\n',
  );
  writeFileSync(join(root, "src/tests.rs"), "#[test] fn works() {}\n");
  writeFileSync(
    join(root, "Cargo.toml"),
    '[package]\nname = "fixture"\nversion = "1.0.0"\n',
  );
  git(["add", "."]);
  git(["commit", "-qm", "release"]);
  git(["tag", "-a", "-m", "release", "v1.0.0"]);
  const releaseCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim();
  writeFileSync(
    join(root, "src/tests.rs"),
    "#[test] fn works() { assert_eq!(1, 1); }\n",
  );
  git(["add", "."]);
  git(["commit", "-qm", "owner tests"]);
  const candidateCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim();
  const parity = {
    tag: "v1.0.0",
    releaseCommit,
    candidateCommit,
    assetsSha256: "a".repeat(64),
    testFile: {
      path: "src/tests.rs",
      releaseMode: "100644",
      releaseBlob: execFileSync(
        "git",
        ["rev-parse", `${releaseCommit}:src/tests.rs`],
        { cwd: root, encoding: "utf8" },
      ).trim(),
      candidateMode: "100644",
      candidateBlob: execFileSync(
        "git",
        ["rev-parse", `${candidateCommit}:src/tests.rs`],
        { cwd: root, encoding: "utf8" },
      ).trim(),
    },
    unchangedTreeSha256: "",
    binding: {
      modulePath: "src/lib.rs",
      moduleBlob: execFileSync(
        "git",
        ["rev-parse", `${candidateCommit}:src/lib.rs`],
        { cwd: root, encoding: "utf8" },
      ).trim(),
      testPath: "src/tests.rs",
    },
  };
  const canonicalTree = execFileSync(
    "git",
    ["ls-tree", "-r", "--full-tree", "-z", releaseCommit],
    { cwd: root, encoding: "utf8" },
  )
    .split("\0")
    .filter(Boolean)
    .map((entry) => {
      const match = entry.match(
        /^([0-7]{6}) (blob|commit) ([0-9a-f]{40})\t([\s\S]+)$/,
      );
      if (match[4] === "src/tests.rs") return null;
      return `${match[1]} ${match[2]} ${match[3]}\t${match[4]}\n`;
    })
    .filter(Boolean)
    .sort()
    .join("");
  parity.unchangedTreeSha256 = createHash("sha256")
    .update(canonicalTree)
    .digest("hex");
  parity.assetsSha256 = createHash("sha256")
    .update(
      JSON.stringify({
        version: "1.0.0",
        assets: [
          {
            hostKey: "linux",
            archive: "fixture.tar.gz",
            sha256: "a".repeat(64),
          },
        ],
      }),
    )
    .digest("hex");
  return { root, git, releaseCommit, candidateCommit, parity };
}

function sourceParitySetFixture({ includeNewTest = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "module-source-parity-set-"));
  const git = (args) =>
    execFileSync("git", args, { cwd: root, stdio: "ignore" });
  git(["init", "-q"]);
  git(["config", "user.email", "pin-check@example.test"]);
  git(["config", "user.name", "Pin Check"]);
  git(["config", "commit.gpgsign", "false"]);
  git(["config", "tag.gpgsign", "false"]);
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(
    join(root, "Cargo.toml"),
    '[package]\nname = "fixture"\nversion = "1.0.0"\n',
  );
  writeFileSync(
    join(root, "src/lib.rs"),
    "pub mod alpha;\npub mod beta;\npub mod gamma;\npub mod future;\n",
  );

  const paths = [
    "src/alpha_tests.rs",
    "src/beta_tests.rs",
    "src/gamma_tests.rs",
  ];
  const modulePaths = ["src/alpha.rs", "src/beta.rs", "src/gamma.rs"];
  for (let index = 0; index < paths.length; index += 1) {
    writeFileSync(
      join(root, modulePaths[index]),
      `#[cfg(test)]\n#[path = "${paths[index].split("/").at(-1)}"]\nmod tests;\npub fn value() -> u8 { ${index} }\n`,
    );
    writeFileSync(
      join(root, paths[index]),
      `#[test] fn works_${index}() {}\n`,
    );
  }
  const futurePath = "src/future_tests.rs";
  const futureModulePath = "src/future.rs";
  writeFileSync(
    join(root, futureModulePath),
    '#[cfg(test)]\n#[path = "future_tests.rs"]\nmod tests;\npub fn future() {}\n',
  );
  git(["add", "."]);
  git(["commit", "-qm", "release"]);
  git(["tag", "-a", "-m", "release", "v1.0.0"]);
  const releaseCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim();
  for (let index = 0; index < paths.length; index += 1) {
    writeFileSync(
      join(root, paths[index]),
      `#[test] fn works_${index}() { assert_eq!(${index}, ${index}); }\n`,
    );
  }
  if (includeNewTest) {
    writeFileSync(join(root, futurePath), "#[test] fn future_test() {}\n");
  }
  git(["add", "."]);
  git(["commit", "-qm", "owner test set"]);
  const candidateCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim();
  const blobAt = (commit, path) => {
    try {
      return execFileSync("git", ["rev-parse", `${commit}:${path}`], {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
    } catch {
      return undefined;
    }
  };
  const testPaths = includeNewTest ? [...paths, futurePath] : paths;
  const testFiles = testPaths.map((path) => ({
    path,
    releaseMode: blobAt(releaseCommit, path) ? "100644" : null,
    releaseBlob: blobAt(releaseCommit, path) ?? null,
    candidateMode: "100644",
    candidateBlob: blobAt(candidateCommit, path),
  }));
  const bindingPaths = includeNewTest
    ? [...modulePaths, futureModulePath]
    : modulePaths;
  const bindings = bindingPaths.map((modulePath, index) => ({
    modulePath,
    moduleBlob: blobAt(candidateCommit, modulePath),
    testPath: testPaths[index],
  }));
  const canonicalTree = execFileSync(
    "git",
    ["ls-tree", "-r", "--full-tree", "-z", releaseCommit],
    { cwd: root, encoding: "utf8" },
  )
    .split("\0")
    .filter(Boolean)
    .map((entry) => {
      const match = entry.match(
        /^([0-7]{6}) (blob|commit) ([0-9a-f]{40})\t([\s\S]+)$/,
      );
      if (testPaths.includes(match[4])) return null;
      return `${match[1]} ${match[2]} ${match[3]}\t${match[4]}\n`;
    })
    .filter(Boolean)
    .sort()
    .join("");
  const parity = {
    tag: "v1.0.0",
    releaseCommit,
    candidateCommit,
    assetsSha256: createHash("sha256")
      .update(
        JSON.stringify({
          version: "1.0.0",
          assets: [
            {
              hostKey: "linux",
              archive: "fixture.tar.gz",
              sha256: "a".repeat(64),
            },
          ],
        }),
      )
      .digest("hex"),
    testFiles,
    unchangedTreeSha256: createHash("sha256").update(canonicalTree).digest("hex"),
    bindings,
  };
  return {
    root,
    git,
    releaseCommit,
    candidateCommit,
    parity,
    paths: testPaths,
  };
}
const provider = {
  id: "tinyx-lang",
  version: "0.2.4",
  releaseUrl: "https://github.com/acme/tinyx-lang/releases/tag/v0.2.4",
  lock: { repo: "acme/tinyx-lang", version: "0.2.4", built_against: SHA_A },
  sourceSubmodule: "vendor/tinyx",
  sourceHead: SHA_A,
};

test("source parity proof accepts only its exact clean test-only Git pair", () => {
  const fixture = sourceParityFixture();
  try {
    const result = validateTestOnlySourceParity({
      repoPath: fixture.root,
      version: "1.0.0",
      assets: [
        { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
      ],
      actualHead: fixture.candidateCommit,
      proof: fixture.parity,
    });
    assert.equal(result.ok, true, result.message);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("source parity proof accepts an explicit set of exact test-only files", () => {
  const fixture = sourceParitySetFixture();
  try {
    const result = validateTestOnlySourceParity({
      repoPath: fixture.root,
      version: "1.0.0",
      assets: [
        { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
      ],
      actualHead: fixture.candidateCommit,
      proof: fixture.parity,
    });
    assert.equal(result.ok, true, result.message);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test(
  "multi-file source parity attests an explicitly bound test file added after release",
  () => {
  const fixture = sourceParitySetFixture({ includeNewTest: true });
  try {
    const result = validateTestOnlySourceParity({
      repoPath: fixture.root,
      version: "1.0.0",
      assets: [
        { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
      ],
      actualHead: fixture.candidateCommit,
      proof: fixture.parity,
    });
    assert.equal(result.ok, true, result.message);
    assert.equal(fixture.parity.testFiles.at(-1).releaseMode, null);
    assert.equal(fixture.parity.testFiles.at(-1).releaseBlob, null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
  },
);

test("multi-file source parity rejects omitted files, extra changes, and widened descendants", () => {
  const assets = [
    { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
  ];
  const validate = (
    fixture,
    proof = fixture.parity,
    actualHead = fixture.candidateCommit,
  ) =>
    validateTestOnlySourceParity({
      repoPath: fixture.root,
      version: "1.0.0",
      assets,
      actualHead,
      proof,
    });
  for (const changedPath of ["Cargo.toml", "src/lib.rs", "src/extra.rs"]) {
    const fixture = sourceParitySetFixture();
    try {
      writeFileSync(join(fixture.root, changedPath), "unattested source change\n");
      fixture.git(["add", changedPath]);
      fixture.git(["commit", "-qm", `change ${changedPath}`]);
      const head = execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: fixture.root,
        encoding: "utf8",
      }).trim();
      const result = validate(
        fixture,
        { ...fixture.parity, candidateCommit: head },
        head,
      );
      assert.equal(result.ok, false, `${changedPath} was accepted`);
      assert.match(
        result.message,
        /other source, manifest, lock, build input, or gitlink differs/,
      );
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }

  const fixture = sourceParitySetFixture();
  try {
    const omitted = validate(fixture, {
      ...fixture.parity,
      testFiles: fixture.parity.testFiles.slice(0, 2),
      bindings: fixture.parity.bindings.slice(0, 2),
    });
    assert.equal(omitted.ok, false, "a changed test file was omitted from the proof set");
    assert.match(
      omitted.message,
      /other source, manifest, lock, build input, or gitlink differs/,
    );

    const duplicate = validate(fixture, {
      ...fixture.parity,
      testFiles: [...fixture.parity.testFiles, fixture.parity.testFiles[0]],
      bindings: [...fixture.parity.bindings, fixture.parity.bindings[0]],
    });
    assert.equal(duplicate.ok, false, "duplicate proof paths were accepted");
    assert.match(duplicate.message, /unique safe Rust source paths/);

    const widenedFile = validate(fixture, {
      ...fixture.parity,
      testFiles: fixture.parity.testFiles.map((file, index) =>
        index === 0 ? { ...file, candidateMode: "100755" } : file,
      ),
    });
    assert.equal(widenedFile.ok, false, "executable test path was accepted");
    assert.match(widenedFile.message, /regular non-executable/);

    const ambiguousShape = validate(fixture, {
      ...fixture.parity,
      testFile: fixture.parity.testFiles[0],
      binding: fixture.parity.bindings[0],
    });
    assert.equal(ambiguousShape.ok, false, "mixed single and set proof shapes were accepted");
    assert.match(
      ambiguousShape.message,
      /proof fields are missing or unknown/,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("the existing exact-drift exemption is proof-checked and keeps ahead visible", () => {
  const fixture = sourceParityFixture();
  try {
    const actual = execFileSync(
      "git",
      ["describe", "--tags", "--abbrev=8", "HEAD"],
      { cwd: fixture.root, encoding: "utf8" },
    ).trim();
    const exemption = {
      id: "tinyx",
      expect: actual,
      reason: "reviewed test-only owner commit",
      sourceParity: fixture.parity,
    };
    const sourceParityContext = {
      repoPath: fixture.root,
      assets: [
        { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
      ],
      actualHead: fixture.candidateCommit,
    };
    const accepted = classifyPin({
      ...base,
      version: "1.0.0",
      actual,
      exemption,
      sourceParityContext,
    });
    assert.equal(accepted.ok, true);
    assert.equal(accepted.kind, "exempt");
    const status = classifyStatus({
      ...base,
      version: "1.0.0",
      actual,
      exemption,
      sourceParityContext,
    });
    assert.equal(status.kind, "exempt");
    assert.ok(
      status.message.includes(actual),
      "the advisory status keeps the actual ahead description visible",
    );
    const altered = classifyPin({
      ...base,
      version: "1.0.0",
      actual,
      exemption: {
        ...exemption,
        sourceParity: { ...fixture.parity, candidateCommit: "f".repeat(40) },
      },
      sourceParityContext,
    });
    assert.equal(altered.ok, false);
    assert.equal(altered.kind, "invalid-source-parity");
    const missing = classifyPin({
      ...base,
      version: "1.0.0",
      actual,
      exemption: { ...exemption, sourceParity: null },
      sourceParityContext,
    });
    assert.equal(
      missing.ok,
      false,
      "a malformed present proof must not fall back to a reason-only exemption",
    );
    assert.equal(missing.kind, "invalid-source-parity");
    const invalidStatus = classifyStatus({
      ...base,
      version: "1.0.0",
      actual,
      exemption: { ...exemption, sourceParity: null },
      sourceParityContext,
    });
    assert.notEqual(invalidStatus.kind, "exempt");
    assert.match(invalidStatus.message, /source-parity exemption is invalid/);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("source parity proof rejects production, manifest, build, lock, and unknown changes", () => {
  for (const changedFile of [
    "src/lib.rs",
    "Cargo.toml",
    "build.rs",
    "Cargo.lock",
    "unknown.txt",
  ]) {
    const fixture = sourceParityFixture();
    try {
      writeFileSync(join(fixture.root, changedFile), "changed\n");
      fixture.git(["add", changedFile]);
      fixture.git(["commit", "-qm", `change ${changedFile}`]);
      const head = execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: fixture.root,
        encoding: "utf8",
      }).trim();
      const result = validateTestOnlySourceParity({
        repoPath: fixture.root,
        version: "1.0.0",
        assets: [
          {
            hostKey: "linux",
            archive: "fixture.tar.gz",
            sha256: "a".repeat(64),
          },
        ],
        actualHead: head,
        proof: fixture.parity,
      });
      assert.equal(result.ok, false, `${changedFile} was incorrectly accepted`);
      const widenedCandidate = validateTestOnlySourceParity({
        repoPath: fixture.root,
        version: "1.0.0",
        assets: [
          {
            hostKey: "linux",
            archive: "fixture.tar.gz",
            sha256: "a".repeat(64),
          },
        ],
        actualHead: head,
        proof: { ...fixture.parity, candidateCommit: head },
      });
      assert.equal(
        widenedCandidate.ok,
        false,
        `${changedFile} was accepted after widening the attested full head`,
      );
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("source parity proof rejects descendants, dirty files, asset drift, renames, symlinks, and gitlinks", () => {
  const assets = [
    { hostKey: "linux", archive: "fixture.tar.gz", sha256: "a".repeat(64) },
  ];
  const validate = (fixture, overrides = {}) =>
    validateTestOnlySourceParity({
      repoPath: fixture.root,
      version: "1.0.0",
      assets,
      actualHead: fixture.candidateCommit,
      proof: fixture.parity,
      ...overrides,
    });

  // A descendant cannot inherit the pair, even when the proof is widened: its
  // candidate test blob must still match the exact attested blob.
  {
    const f = sourceParityFixture();
    try {
      writeFileSync(join(f.root, "src/tests.rs"), "#[test] fn later() {}\n");
      f.git(["add", "."]);
      f.git(["commit", "-qm", "later test change"]);
      const head = execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: f.root,
        encoding: "utf8",
      }).trim();
      assert.match(
        validate(f, { actualHead: head }).message,
        /HEAD is not the exact attested candidate/,
      );
      assert.match(
        validate(f, {
          actualHead: head,
          proof: { ...f.parity, candidateCommit: head },
        }).message,
        /test file path, mode, or blob/,
      );
    } finally {
      rmSync(f.root, { recursive: true, force: true });
    }
  }

  // Metadata mutants stay at the exact candidate HEAD, so each reaches and
  // proves its own invariant rather than failing the earlier HEAD check.
  for (const [name, overrides, diagnostic] of [
    [
      "asset map",
      { assets: [] },
      /registry version or artifact asset map changed/,
    ],
    ["registry version", { version: "1.1.0" }, /release tag does not match/],
    [
      "unknown field",
      { proof: { unknown: true } },
      /proof fields are missing or unknown/,
    ],
    [
      "whole-tree digest",
      { proof: { unchangedTreeSha256: "f".repeat(64) } },
      /other source, manifest, lock, build input, or gitlink differs/,
    ],
    [
      "mode",
      { proof: { testFile: { candidateMode: "100755" } } },
      /regular non-executable/,
    ],
    [
      "binding blob",
      { proof: { binding: { moduleBlob: "f".repeat(40) } } },
      /module declaration source differs/,
    ],
  ]) {
    const f = sourceParityFixture();
    try {
      const proof = {
        ...f.parity,
        ...overrides.proof,
        testFile: { ...f.parity.testFile, ...overrides.proof?.testFile },
        binding: { ...f.parity.binding, ...overrides.proof?.binding },
      };
      const result = validate(f, { ...overrides, proof });
      assert.match(
        result.message,
        diagnostic,
        `${name} mutant failed for the wrong reason: ${result.message}`,
      );
    } finally {
      rmSync(f.root, { recursive: true, force: true });
    }
  }

  for (const kind of [
    "dirty-untracked",
    "dirty-tracked",
    "symlink",
    "rename",
    "gitlink",
  ]) {
    const f = sourceParityFixture();
    let nestedRoot;
    try {
      if (kind === "dirty-untracked" || kind === "dirty-tracked") {
        const path = kind === "dirty-tracked" ? "src/lib.rs" : "production.rs";
        writeFileSync(join(f.root, path), "uncommitted production input\n");
        assert.match(
          validate(f).message,
          /module worktree is dirty/,
          `${kind} must fail at the cleanliness check`,
        );
        continue;
      }
      if (kind === "symlink") {
        unlinkSync(join(f.root, "src/tests.rs"));
        symlinkSync("lib.rs", join(f.root, "src/tests.rs"));
        f.git(["add", "-A"]);
      } else if (kind === "rename") {
        f.git(["mv", "src/tests.rs", "src/renamed.rs"]);
      } else {
        nestedRoot = mkdtempSync(
          join(tmpdir(), "module-source-parity-nested-"),
        );
        execFileSync("git", ["init", "-q"], {
          cwd: nestedRoot,
          stdio: "ignore",
        });
        execFileSync(
          "git",
          ["config", "user.email", "pin-check@example.test"],
          { cwd: nestedRoot, stdio: "ignore" },
        );
        execFileSync("git", ["config", "user.name", "Pin Check"], {
          cwd: nestedRoot,
          stdio: "ignore",
        });
        writeFileSync(join(nestedRoot, "module.rs"), "pub fn nested() {}\n");
        execFileSync("git", ["add", "."], { cwd: nestedRoot, stdio: "ignore" });
        execFileSync("git", ["commit", "-qm", "nested module"], {
          cwd: nestedRoot,
          stdio: "ignore",
        });
        f.git([
          "-c",
          "protocol.file.allow=always",
          "submodule",
          "add",
          nestedRoot,
          "nested-module",
        ]);
      }
      f.git(["commit", "-qm", `${kind} change`]);
      const head = execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: f.root,
        encoding: "utf8",
      }).trim();
      // Bind the actual new head but deliberately keep the original path/blob,
      // mode and whole-tree claims so the structural mutation is what rejects.
      const result = validate(f, {
        actualHead: head,
        proof: { ...f.parity, candidateCommit: head },
      });
      assert.match(
        result.message,
        kind === "gitlink"
          ? /other source, manifest, lock, build input, or gitlink differs/
          : /test file path, mode, or blob/,
        `${kind} failed for the wrong reason: ${result.message}`,
      );
    } finally {
      rmSync(f.root, { recursive: true, force: true });
      if (nestedRoot) rmSync(nestedRoot, { recursive: true, force: true });
    }
  }

  for (const tagChange of ["moved", "missing"]) {
    const f = sourceParityFixture();
    try {
      f.git(
        tagChange === "moved"
          ? ["tag", "-f", "-a", "-m", "moved", "v1.0.0", f.candidateCommit]
          : ["tag", "-d", "v1.0.0"],
      );
      assert.match(
        validate(f).message,
        tagChange === "moved"
          ? /release tag does not resolve to the attested release commit/
          : /unknown revision/,
        `${tagChange} tag failed for the wrong reason`,
      );
    } finally {
      rmSync(f.root, { recursive: true, force: true });
    }
  }
});

test("a provider released against the host's source pin passes", () => {
  assert.equal(classifyProviderPin(provider).ok, true);
});

test("a provider built against an older source commit fails and says to re-release", () => {
  const v = classifyProviderPin({ ...provider, sourceHead: SHA_B });
  assert.equal(v.ok, false);
  assert.match(v.message, /built against vendor\/tinyx aaaaaaaa/);
  assert.match(v.message, /Re-release/);
});

test("a provider whose registry version or url differs from its lock fails", () => {
  const v = classifyProviderPin({
    ...provider,
    version: "0.2.3",
    releaseUrl: "https://github.com/acme/tinyx-lang/releases/tag/v0.2.3",
  });
  assert.equal(v.ok, false);
  assert.match(v.message, /0\.2\.3/);
  const url = classifyProviderPin({
    ...provider,
    releaseUrl: "https://github.com/other/repo/releases/tag/v0.2.4",
  });
  assert.equal(url.ok, false);
});

test("a provider with no complete lock entry, or a short sha, fails", () => {
  assert.equal(classifyProviderPin({ ...provider, lock: undefined }).ok, false);
  const v = classifyProviderPin({
    ...provider,
    lock: { ...provider.lock, built_against: "aaaaaaaa" },
  });
  assert.equal(v.ok, false);
  assert.match(v.message, /40-hex/);
});

test("parseRecords reads release_url", () => {
  const recs = parseRecords(
    'const X: ModuleRecord = ModuleRecord {\n    id: "x",\n    version: "1.0.0",\n    release_url: "https://github.com/a/b/releases/tag/v1.0.0",\n    assets: &[],\n};',
  );
  assert.equal(
    recs.get("X").releaseUrl,
    "https://github.com/a/b/releases/tag/v1.0.0",
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

  const longerAbbreviation = classifyPin({
    ...base,
    actual: "v1.2.3-7-gabc12345",
    exemption,
  });
  assert.equal(longerAbbreviation.ok, true);
  assert.equal(longerAbbreviation.kind, "exempt");

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

test("an unparseable registry fails the gate instead of passing", () => {
  const root = fixtureRoot({
    "crates/openhuman-core/src/modules/registry.rs":
      "// everything here got deleted\n",
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
  const root = fixtureRoot({});
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
    "crates/openhuman-core/src/modules/registry.rs": registry,
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

test("a cfg attribute on an ALL element does not hide the record", () => {
  const src = [
    "pub const ALL: &[ModuleRecord] = &[",
    "    TINYCOMPUTER,",
    '    #[cfg(feature = "documents")]',
    "    TINYDOCS,",
    '    #[cfg(feature = "modules")] // loader only',
    "    TINYBOX,",
    "];",
  ].join("\n");
  assert.deepEqual(parseAllList(src), ["TINYCOMPUTER", "TINYDOCS", "TINYBOX"]);
});
