// Unit tests for scripts/ci/assert-coverage-presence.sh — the hard gate that
// fails when the coverage lane produced no records at all for a changed Rust
// source file (#5613).
//
// Each test pins one clause of the script's `eligible()` filter. Delete the
// corresponding clause and exactly one test here goes red, which is what makes
// the exclusion list a reviewed ratchet rather than a pile of guesses.
//
// The fixtures are throwaway git repos: the script's `--all` mode reads
// `git ls-files`, and `eligible()` stats real paths.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const script = path.join(
  repoRoot,
  "scripts",
  "ci",
  "assert-coverage-presence.sh",
);

/** A source file with a real `fn`, so it is instrumentable. */
const WITH_FN = "pub fn thing() -> u8 {\n    1\n}\n";
/** A barrel module: re-exports only, no `fn`, can never produce a region. */
const NO_FN = "pub mod a;\npub use a::Thing;\n";

/**
 * Build a throwaway repo, write `files`, and run the gate over it.
 *
 * @param {Record<string,string>} files repo-relative path -> contents
 * @param {string[]} coveredPaths paths to emit as `SF:` records
 * @param {string[]} args        args after the lcov path
 * @param {string}   allowlist   contents of the allowlist file, if any
 */
function run(files, coveredPaths, args, allowlist = null) {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-cov-presence-"));
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(cwd, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  execFileSync("git", ["init", "-q", "."], { cwd });
  execFileSync("git", ["add", "-A"], { cwd });

  const lcov = path.join(cwd, "cov.info");
  fs.writeFileSync(
    lcov,
    coveredPaths.map((p) => `SF:${cwd}/${p}\nDA:1,1\nend_of_record\n`).join(""),
  );

  const env = { ...process.env };
  if (allowlist !== null) {
    const listPath = path.join(cwd, "allow.txt");
    fs.writeFileSync(listPath, allowlist);
    env.ALLOWLIST = listPath;
  } else {
    // Default to no allowlist so a fixture never picks up the repo's real one.
    env.ALLOWLIST = path.join(cwd, "absent.txt");
  }

  try {
    const stdout = execFileSync("bash", [script, lcov, ...args], {
      cwd,
      encoding: "utf8",
      env,
    });
    return { status: 0, output: stdout };
  } catch (err) {
    return {
      status: err.status,
      output: `${err.stdout ?? ""}${err.stderr ?? ""}`,
    };
  }
}

test("fails and names a changed file that produced no coverage records", () => {
  const res = run(
    { "src/a/covered.rs": WITH_FN, "src/a/uncompiled.rs": WITH_FN },
    ["src/a/covered.rs"],
    ["--files", "src/a/covered.rs", "src/a/uncompiled.rs"],
  );
  assert.equal(res.status, 1);
  assert.match(
    res.output,
    /src\/a\/uncompiled\.rs produced no coverage records/,
  );
  assert.doesNotMatch(res.output, /file=src\/a\/covered\.rs/);
});

test("passes when every eligible changed file is present in the lcov", () => {
  const res = run(
    { "src/a/covered.rs": WITH_FN },
    ["src/a/covered.rs"],
    ["--files", "src/a/covered.rs"],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /clean/);
});

test("skips barrel modules that declare no fn", () => {
  const res = run({ "src/a/mod.rs": NO_FN }, [], ["--files", "src/a/mod.rs"]);
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("a fn mentioned only inside a line comment does not make a file eligible", () => {
  const res = run(
    { "src/a/mod.rs": "// pub fn documented() {}\npub use x::Y;\n" },
    [],
    ["--files", "src/a/mod.rs"],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("large files stay eligible (regression: pipefail + grep -q SIGPIPE)", () => {
  // The first implementation used `grep -v … | grep -q …`. Under `set -o
  // pipefail` the reader exits at the first match, the writer dies of SIGPIPE,
  // and the pipeline returns 141 — so any file long enough to still be
  // streaming read as "no fn" and was silently skipped. That excluded 299 of
  // 1,377 eligible sources, including the 937-line file this gate was written
  // to catch. The `fn` here is deliberately at the top with bulk after it.
  const big = WITH_FN + "// filler\n".repeat(20000);
  const res = run({ "src/a/big.rs": big }, [], ["--files", "src/a/big.rs"]);
  assert.equal(
    res.status,
    1,
    "a large uncovered source file must still be checked",
  );
  assert.match(res.output, /checked 1 eligible/);
  assert.match(res.output, /src\/a\/big\.rs produced no coverage records/);
});

test("skips test sources, stub.rs, per-OS modules and deleted paths", () => {
  const res = run(
    {
      "src/a/thing_tests.rs": WITH_FN,
      "src/a/thing_test.rs": WITH_FN,
      "src/a/tests.rs": WITH_FN,
      "src/a/test_support.rs": WITH_FN,
      "src/a/tests/helper.rs": WITH_FN,
      "src/a/stub.rs": WITH_FN,
      "src/a/macos.rs": WITH_FN,
      "src/a/windows.rs": WITH_FN,
    },
    [],
    [
      "--files",
      "src/a/thing_tests.rs",
      "src/a/thing_test.rs",
      "src/a/tests.rs",
      "src/a/test_support.rs",
      "src/a/tests/helper.rs",
      "src/a/stub.rs",
      "src/a/macos.rs",
      "src/a/windows.rs",
      "src/a/deleted.rs",
    ],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("skips non-Rust paths, crate roots and src/bin", () => {
  const res = run(
    {
      "src/lib.rs": WITH_FN,
      "src/main.rs": WITH_FN,
      "src/bin/tool.rs": WITH_FN,
      "src/a/README.md": "# doc\n",
    },
    [],
    [
      "--files",
      "src/lib.rs",
      "src/main.rs",
      "src/bin/tool.rs",
      "src/a/README.md",
    ],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("skips families that are uncovered by design", () => {
  const res = run(
    {
      "src/tui/app.rs": WITH_FN,
      "src/openhuman/test_support/reset.rs": WITH_FN,
      "src/openhuman/tools/impl/browser/native_backend.rs": WITH_FN,
    },
    [],
    [
      "--files",
      "src/tui/app.rs",
      "src/openhuman/test_support/reset.rs",
      "src/openhuman/tools/impl/browser/native_backend.rs",
    ],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("normalises .. inside SF: paths so #[path] modules are not false-failed", () => {
  // rustc records the literal string from a `#[path = "../x.rs"]` attribute,
  // so the lcov can name a real, compiled file by a non-canonical path.
  const res = run(
    { "src/a/b.rs": WITH_FN },
    ["src/a/sub/../b.rs"],
    ["--files", "src/a/b.rs"],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /clean/);
});

test("honours the allowlist, ignoring comments and blank lines", () => {
  const files = { "src/a/gated.rs": WITH_FN };
  const bare = run(files, [], ["--files", "src/a/gated.rs"]);
  assert.equal(bare.status, 1, "sanity: unlisted the file must fail");

  const listed = run(
    files,
    [],
    ["--files", "src/a/gated.rs"],
    "# a reason\n\nsrc/a/gated.rs\n",
  );
  assert.equal(listed.status, 0);
  assert.match(listed.output, /checked 0 eligible/);
});

test("--all walks the tracked tree", () => {
  const res = run(
    { "src/a/covered.rs": WITH_FN, "src/a/uncompiled.rs": WITH_FN },
    ["src/a/covered.rs"],
    ["--all"],
  );
  assert.equal(res.status, 1);
  assert.match(res.output, /checked 2 eligible/);
  assert.match(
    res.output,
    /src\/a\/uncompiled\.rs produced no coverage records/,
  );
});

test("--all still enumerates when git ls-files fails (CI dubious-ownership)", () => {
  // Regression for run 32367545922: actions/checkout registers `safe.directory`
  // under a temporarily overridden HOME, so `git ls-files` in a later step dies
  // with `fatal: detected dubious ownership`. Read through a process
  // substitution that yielded an EMPTY candidate list, and the gate reported
  // "clean" having checked ZERO files — the verified-nothing fail-open this
  // script exists to close, reproduced inside it.
  const cwd = fs.mkdtempSync(
    path.join(os.tmpdir(), "openhuman-cov-presence-nogit-"),
  );
  fs.mkdirSync(path.join(cwd, "src", "a"), { recursive: true });
  fs.writeFileSync(path.join(cwd, "src", "a", "uncompiled.rs"), WITH_FN);
  fs.writeFileSync(path.join(cwd, "cov.info"), "");

  // A `git` on PATH that always fails, standing in for dubious ownership.
  const bin = fs.mkdtempSync(path.join(os.tmpdir(), "openhuman-fakegit-"));
  fs.writeFileSync(
    path.join(bin, "git"),
    "#!/usr/bin/env bash\necho 'fatal: detected dubious ownership' >&2\nexit 128\n",
  );
  fs.chmodSync(path.join(bin, "git"), 0o755);

  let status = 0;
  let output = "";
  try {
    output = execFileSync(
      "bash",
      [script, path.join(cwd, "cov.info"), "--all"],
      {
        cwd,
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: `${bin}:${process.env.PATH}`,
          ALLOWLIST: path.join(cwd, "absent.txt"),
        },
      },
    );
  } catch (err) {
    status = err.status;
    output = `${err.stdout ?? ""}${err.stderr ?? ""}`;
  }

  assert.match(output, /falling back to a filesystem walk/);
  assert.equal(
    status,
    1,
    `the uncompiled file must still be found, got:\n${output}`,
  );
  assert.match(output, /checked 1 eligible/);
  assert.match(output, /src\/a\/uncompiled\.rs produced no coverage records/);
});

test("--all refuses to report success when it checked nothing", () => {
  // Zero eligible files in whole-tree mode means the walk broke, not that there
  // is nothing to verify. Exit 2 (environment error), not 0.
  const cwd = fs.mkdtempSync(
    path.join(os.tmpdir(), "openhuman-cov-presence-empty-"),
  );
  fs.writeFileSync(path.join(cwd, "cov.info"), "");
  execFileSync("git", ["init", "-q", "."], { cwd });
  let status = 0;
  let output = "";
  try {
    execFileSync("bash", [script, path.join(cwd, "cov.info"), "--all"], {
      cwd,
      encoding: "utf8",
      env: { ...process.env, ALLOWLIST: path.join(cwd, "absent.txt") },
    });
  } catch (err) {
    status = err.status;
    output = `${err.stdout ?? ""}${err.stderr ?? ""}`;
  }
  assert.equal(status, 2, `expected exit 2, got ${status}:\n${output}`);
  assert.match(output, /checked 0 eligible files/);
});

test("--files may legitimately check nothing and still pass", () => {
  // The counterpart: a PR touching only test sources has nothing to verify, and
  // that is not an error.
  const res = run(
    { "src/a/thing_tests.rs": WITH_FN },
    [],
    ["--files", "src/a/thing_tests.rs"],
  );
  assert.equal(res.status, 0);
  assert.match(res.output, /checked 0 eligible/);
});

test("exits 2 on a missing lcov file and on bad usage", () => {
  const cwd = fs.mkdtempSync(
    path.join(os.tmpdir(), "openhuman-cov-presence-usage-"),
  );
  const bad = (args) => {
    try {
      execFileSync("bash", [script, ...args], { cwd, encoding: "utf8" });
      return 0;
    } catch (err) {
      return err.status;
    }
  };
  assert.equal(bad([path.join(cwd, "nope.info"), "--all"]), 2);
  fs.writeFileSync(path.join(cwd, "cov.info"), "");
  assert.equal(bad([path.join(cwd, "cov.info"), "--bogus"]), 2);
  assert.equal(bad([path.join(cwd, "cov.info")]), 2);
});
