import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
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

test("private workflow pins checkout and image without a compile or artifact cache", () => {
  const workflow = readFileSync(
    new URL("../../.github/workflows/ci-lanes.yml", import.meta.url),
    "utf8",
  );
  const privateJob = workflow.split("  # ── hosted:")[0];
  assert.match(privateJob, /group: openhuman-ex63/);
  assert.match(privateJob, /labels: \[self-hosted, openhuman-ex63\]/);
  assert.match(
    privateJob,
    /tinyhumansai\/\.github\/actions\/checkout@[a-f0-9]{40}/,
  );
  assert.match(privateJob, /submodules: recursive/);
  assert.match(privateJob, /exclude-submodules: wiki/);
  assert.match(privateJob, /fetch-depth: 0/);
  assert.match(privateJob, /submodule-depth: 0/);
  assert.match(privateJob, /allow-unsafe-pr-checkout: true/);
  assert.doesNotMatch(
    privateJob,
    /Swatinem\/rust-cache|sccache|actions\/cache|tinymold|TINYMOLD/i,
  );
  assert.ok(
    privateJob.indexOf("verify-runner.sh") <
      privateJob.indexOf("- name: Start lanes"),
  );
});

test("private routing requires both trusted identities and retains outsider hosted fallback", () => {
  const root = new URL("../../.github/workflows/", import.meta.url);
  const route = readFileSync(new URL("ci-fast.yml", root), "utf8");
  const hosted = readFileSync(new URL("ci-fast-hosted.yml", root), "utf8");
  assert.match(route, /pull_request_target:/);
  assert.match(route, /is_member "\$\{AUTHOR\}" && is_member "\$\{ACTOR\}"/);
  assert.match(route, /contents: read/);
  assert.match(route, /needs.route.outputs.member == 'true'/);
  assert.match(hosted, /profile: hosted/);
  assert.doesNotMatch(route, /contents: write|secrets: inherit/);
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

test("membership gate routes mixed identities, outsiders and bots away from EX63", () => {
  const route = readFileSync(
    new URL("../../.github/workflows/ci-fast.yml", import.meta.url),
    "utf8",
  );
  const body = route
    .split("        run: |\n")[1]
    .split("\n  lanes:")[0]
    .split("\n")
    .map((line) => line.slice(10))
    .join("\n");
  const dir = mkdtempSync(join(tmpdir(), "ci-membership-"));
  try {
    writeFileSync(
      join(dir, "curl"),
      '#!/usr/bin/env bash\nurl="${!#}"\ncase "$url" in */members/alice|*/members/bob) echo 204;; *) echo 404;; esac\n',
      { mode: 0o755 },
    );
    for (const [author, actor, association, token, expected, event, draft] of [
      ["alice", "bob", "MEMBER", "fixture", true],
      // Draft PRs stay off the EX63, even an org member's.
      ["alice", "bob", "MEMBER", "fixture", false, undefined, "true"],
      ["alice", "alice", "MEMBER", "", false, undefined, "true"],
      ["alice", "bob", "MEMBER", "fixture", true, undefined, "false"],
      ["alice", "outsider", "MEMBER", "fixture", false],
      ["outsider", "alice", "NONE", "fixture", false],
      ["dependabot[bot]", "dependabot[bot]", "NONE", "fixture", false],
      ["alice", "alice", "MEMBER", "", true],
      ["alice", "bob", "MEMBER", "", false],
      // A manual dispatch has no PR author: only the dispatcher is checked,
      // and only when the membership token can check it.
      ["", "alice", "", "fixture", true, "workflow_dispatch"],
      ["", "outsider", "", "fixture", false, "workflow_dispatch"],
      ["", "alice", "", "", false, "workflow_dispatch"],
    ]) {
      const output = join(dir, "output");
      writeFileSync(output, "");
      const result = spawnSync("bash", ["-c", body], {
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: `${dir}:${process.env.PATH}`,
          ORG: "fixture",
          AUTHOR: author,
          ACTOR: actor,
          ASSOCIATION: association,
          MEMBERSHIP_TOKEN: token,
          GITHUB_OUTPUT: output,
          GITHUB_EVENT_NAME: event ?? "pull_request_target",
          DRAFT: draft ?? "",
        },
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(
        readFileSync(output, "utf8").trim(),
        `member=${expected}`,
        `${author}/${actor}`,
      );
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("private validation has a measured budget without suppressing artifacts", () => {
  const workflow = readFileSync(
    new URL("../../.github/workflows/ci-lanes.yml", import.meta.url),
    "utf8",
  ).split("  # ── hosted:")[0];
  const job = workflow.slice(
    workflow.indexOf("  ex63:"),
    workflow.indexOf("    steps:"),
  );
  assert.match(job, /timeout-minutes: 180/);
  const upload = workflow.slice(
    workflow.indexOf("      - name: Upload lane logs, timings and coverage"),
  );
  assert.match(upload, /if: always\(\)/);
  assert.doesNotMatch(upload, /success\(\)/);
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
