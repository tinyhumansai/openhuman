import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  rmSync,
  readFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { classifyElf } from "../ci/self-hosted/verify-linkage.mjs";
import {
  assertGithubHostedRunners,
  jobBlock,
} from "../ci/workflow-contract.mjs";

const helper = new URL("../ci/self-hosted/verify-runner.sh", import.meta.url);
const pin = readFileSync(
  new URL("../ci/self-hosted/guest-image.txt", import.meta.url),
  "utf8",
).trim();

test("private runner rejects missing, stale and unqualified guest image metadata", () => {
  const dir = mkdtempSync(join(tmpdir(), "guest-contract-"));
  try {
    const metadata = join(dir, "build-info");
    const run = () =>
      spawnSync("bash", [helper.pathname, "image"], {
        encoding: "utf8",
        env: { ...process.env, CI_IMAGE_BUILD_INFO: metadata },
      });
    assert.notEqual(run().status, 0);
    writeFileSync(metadata, "digest=sha256:stale\n");
    assert.notEqual(run().status, 0);
    writeFileSync(metadata, `image=${pin}\n`);
    assert.notEqual(run().status, 0);
    writeFileSync(metadata, `digest=${pin.split("@")[1]}\n`);
    assert.equal(run().status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("every PR runs the hosted lanes, with nothing routing it to another runner", () => {
  const root = new URL("../../.github/workflows/", import.meta.url);
  const hosted = readFileSync(new URL("ci-fast-hosted.yml", root), "utf8");
  const lanes = readFileSync(new URL("ci-lanes.yml", root), "utf8");
  assert.match(hosted, /^  pull_request:$/m);
  assert.match(hosted, /uses: \.\/\.github\/workflows\/ci-lanes\.yml/);
  // A condition or dependency on the lanes job can skip them, and the gate
  // never counts a skipped job as a pass. jobBlock throws when the job is
  // missing, so renaming it cannot make these checks vacuous.
  const lanesJob = jobBlock(hosted, "lanes");
  assert.doesNotMatch(lanesJob, /^    (if|needs):/m);
  assert.doesNotMatch(hosted, /ci-fast-route/);
  assert.equal(existsSync(new URL("ci-fast.yml", root)), false);
  assertGithubHostedRunners(lanes);
  assert.doesNotMatch(lanes, /openhuman-ex63/);
});

test("the workflow contract helpers fail loudly instead of passing vacuously", () => {
  const renamed = "jobs:\n  gate:\n    if: false\n    name: Lanes\n";
  assert.throws(() => jobBlock(renamed, "lanes"), /no job "lanes"/);
  // The block of a job stops at the next job, so a sibling's condition is not
  // attributed to it.
  const two = "jobs:\n  lanes:\n    name: L\n  other:\n    if: false\n";
  assert.doesNotMatch(jobBlock(two, "lanes"), /if:/);
  assert.match(jobBlock(two, "other"), /if: false/);
  const wrap = (runsOn) => `jobs:\n  a:\n    runs-on: ${runsOn}\n`;
  for (const ok of ["ubuntu-latest", "ubuntu-22.04", "windows-2022", "macos-15"])
    assert.doesNotThrow(() => assertGithubHostedRunners(wrap(ok)), ok);
  for (const bad of [
    "self-hosted",
    "[ self-hosted, linux ]",
    "[self-hosted]",
    "openhuman-ex63",
    "{ group: private }",
    "${{ inputs.runner }}",
  ])
    assert.throws(() => assertGithubHostedRunners(wrap(bad)), /runs-on/, bad);
  assert.throws(
    () => assertGithubHostedRunners("jobs:\n  a:\n    runs-on:\n      group: x\n"),
    /runs-on/,
  );
  assert.throws(
    () => assertGithubHostedRunners("jobs:\n  a:\n    name: no runner\n"),
    /no runs-on/,
  );
});

test("mold proof distinguishes instrumented binaries from ordinary executables", () => {
  assert.deepEqual(
    classifyElf(
      " [1] .interp PROGBITS\n [2] __llvm_prf_cnts PROGBITS",
      "mold 2.40.4",
    ),
    {
      executable: true,
      instrumented: true,
      mold: true,
    },
  );
  assert.deepEqual(classifyElf(" [1] .interp PROGBITS", "GCC 15"), {
    executable: true,
    instrumented: false,
    mold: false,
  });
  assert.equal(
    classifyElf(" [1] .text PROGBITS", "mold 2.40.4").executable,
    false,
  );
});

test("private LLVM preflight rejects missing tools before publishing environment", () => {
  const source = readFileSync(helper, "utf8");
  const definition = source.match(
    /^verify_llvm_tools\(\) \{\n[\s\S]*?^\}/m,
  )?.[0];
  assert.ok(
    definition,
    "LLVM preflight must validate the pinned toolchain tools",
  );
  const dir = mkdtempSync(join(tmpdir(), "guest LLVM tools "));
  try {
    const libdir = join(dir, "pinned toolchain", "lib");
    const bindir = join(dir, "pinned toolchain", "bin");
    mkdirSync(bindir, { recursive: true });
    const envFile = join(dir, "github-env");
    const log = join(dir, "rustup-args");
    writeFileSync(
      join(dir, "rustup"),
      `#!/usr/bin/env bash
printf "%s\\n" "$*" >> "$RUSTUP_LOG"
[[ "$*" == "run pinned-fixture rustc --print target-libdir" ]] || exit 91
[[ "$LOOKUP_FAIL" != 1 ]] || exit 92
printf "%s\\n" "$FIXTURE_LIBDIR"
`,
      { mode: 0o755 },
    );
    for (const [name, covMode, profdataMode, lookupFail, succeeds] of [
      ["both tools executable", 0o755, 0o755, false, true],
      ["llvm-cov absent", null, 0o755, false, false],
      ["llvm-profdata absent", 0o755, null, false, false],
      ["llvm-cov not executable", 0o644, 0o755, false, false],
      ["llvm-profdata not executable", 0o755, 0o644, false, false],
      ["toolchain lookup fails", 0o755, 0o755, true, false],
    ]) {
      for (const [tool, mode] of [
        ["llvm-cov", covMode],
        ["llvm-profdata", profdataMode],
      ]) {
        const path = join(bindir, tool);
        rmSync(path, { force: true });
        if (mode !== null) writeFileSync(path, "#!/bin/sh\nexit 0\n", { mode });
      }
      writeFileSync(envFile, "");
      writeFileSync(log, "");
      const result = spawnSync(
        "bash",
        [
          "-c",
          `set -euo pipefail\n${definition}\nverify_llvm_tools pinned-fixture`,
        ],
        {
          encoding: "utf8",
          env: {
            ...process.env,
            PATH: `${dir}:${process.env.PATH}`,
            GITHUB_ENV: envFile,
            FIXTURE_LIBDIR: libdir,
            RUSTUP_LOG: log,
            LOOKUP_FAIL: lookupFail ? "1" : "0",
          },
        },
      );
      assert.equal(result.status === 0, succeeds, `${name}: ${result.stderr}`);
      assert.equal(
        readFileSync(envFile, "utf8"),
        succeeds ? "CARGO_LLVM_COV_SETUP=no\n" : "",
        name,
      );
      assert.equal(
        readFileSync(log, "utf8"),
        "run pinned-fixture rustc --print target-libdir\n",
        "preflight must use the pinned toolchain without installing components",
      );
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  const invocation = source.indexOf('\nverify_llvm_tools "$channel"\n');
  assert.ok(invocation > source.indexOf("channel="));
  assert.ok(invocation < source.indexOf('echo "RUSTUP_TOOLCHAIN=$channel"'));
  assert.doesNotMatch(source, /TINYMOLD|tinymold/);
});
