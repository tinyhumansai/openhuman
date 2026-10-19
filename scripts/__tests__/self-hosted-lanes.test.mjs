// Self-test for the CI lane flow: scripts/ci/self-hosted/{lanes-plan,lanes}.mjs
// (run by ci-fast.yml / ci-fast-hosted.yml through ci-lanes.yml).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  AREA_ENV,
  HOSTED_GROUPS,
  areasFromEnv,
  buildPlan,
  hostedMatrix,
  selectLanes,
  validatePlan,
} from "../ci/self-hosted/lanes-plan.mjs";
import {
  PrioritySemaphore,
  processTable,
  treeRssMiB,
  Runner,
  defaultHeavySlots,
  foldLine,
  gatingFailures,
  renderLaneTable,
  orderProblems,
  parseArgs,
  renderSummary,
} from "../ci/self-hosted/lanes.mjs";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const ciLanes = fs.readFileSync(
  path.join(repoRoot, ".github", "workflows", "ci-lanes.yml"),
  "utf8",
);

const ALL = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, true]));
const NONE = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, false]));
const EX63_ENV = { CI_SCRATCH_DIR: "/scratch" };

test("EX63 verifies preinstalled tools before cache setup and validation", () => {
  const privateJob = ciLanes.slice(
    ciLanes.indexOf("  ex63:"),
    ciLanes.indexOf("  plan:"),
  );
  const verify = privateJob.indexOf("- name: Verify private runner contract");
  const cache = privateJob.indexOf("- name: Setup Tinymold");
  const detect = privateJob.indexOf("- name: Detect changed areas");
  const start = privateJob.indexOf("- name: Start lanes");
  assert.ok(verify >= 0 && cache > verify && detect > cache && start > detect);
  assert.match(privateJob, /bash scripts\/ci\/self-hosted\/verify-runner\.sh/);
  assert.doesNotMatch(privateJob, /setup-python|apt-get|JQ_SHA256|sccache/);
  assert.match(privateJob, /Save Tinymold cache[\s\S]*continue-on-error: true/);
});

const plans = () => [
  buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV }),
  buildPlan({ profile: "hosted", areas: ALL }),
];
const allRuns = (plan) => plan.lanes.flatMap((l) => l.checks.map((c) => c.run));

test("both profiles build a plan whose dependencies resolve and cannot deadlock", () => {
  for (const plan of plans()) {
    assert.deepEqual(validatePlan(plan), [], plan.profile);
    assert.deepEqual(orderProblems(plan), [], plan.profile);
  }
});

test("every hosted group resolves on its own, and every hosted lane has exactly one group", () => {
  const plan = buildPlan({ profile: "hosted", areas: ALL });
  for (const g of HOSTED_GROUPS) {
    const sub = selectLanes(plan, g.lanes);
    assert.deepEqual(validatePlan(sub), [], g.group);
    assert.deepEqual(orderProblems(sub), [], g.group);
  }
  const grouped = HOSTED_GROUPS.flatMap((g) => g.lanes).sort();
  assert.deepEqual(grouped, plan.lanes.map((l) => l.name).sort());
});

test("Rust coverage installs the mock backend dependencies without a frontend area", () => {
  for (const profile of ["hosted", "ex63"]) {
    const plan = buildPlan({
      profile,
      areas: { ...NONE, rustCore: true },
      env: profile === "ex63" ? EX63_ENV : {},
    });
    const selected = selectLanes(
      plan,
      profile === "ex63" ? ["frontend", "rust-cov"] : ["rust-cov"],
    );
    assert.deepEqual(validatePlan(selected), []);
    const checks = selected.lanes.flatMap((lane) => lane.checks);
    const install = checks.find((c) => c.name === "pnpm-install");
    assert.equal(install.when, true);
    assert.equal(install.run, "pnpm install --frozen-lockfile");
    assert.ok(
      checks
        .find((c) => c.name === "rust-core-coverage")
        .needs.includes(
          profile === "ex63" ? "frontend:pnpm-install" : "pnpm-install",
        ),
    );
  }
});

test("commands are static: no suite is ever narrowed to the diff", () => {
  for (const plan of plans()) {
    for (const run of allRuns(plan)) {
      assert.doesNotMatch(
        run,
        /\$\{\{|CI_AREA_|git diff|CHANGED|--changed|\brelated\b|--test-files/,
        run,
      );
    }
  }
});

test("ex63 runs the core's unit tests under nextest; hosted keeps cargo's runner", () => {
  for (const plan of plans()) {
    const cov = plan.lanes
      .find((l) => l.name === "rust-cov")
      .checks.find((c) => c.name === "rust-core-coverage");
    assert.equal(
      cov.env.OH_COV_RUNNER,
      plan.profile === "ex63" ? "nextest" : undefined,
    );
  }
  assert.match(
    fs.readFileSync(path.join(repoRoot, ".config/nextest.toml"), "utf8"),
    /\[profile\.ci\][\s\S]*fail-fast = false/,
  );
});

test("a core-only change still installs the node deps rust-core-coverage's mock backend imports", () => {
  const coreOnly = { ...NONE, rustCore: true };
  for (const plan of [
    buildPlan({ profile: "ex63", areas: coreOnly, env: EX63_ENV }),
    buildPlan({ profile: "hosted", areas: coreOnly }),
  ]) {
    const checks = new Map(
      plan.lanes.flatMap((l) =>
        l.checks.map((c) => [`${l.name}:${c.name}`, c]),
      ),
    );
    const cov = checks.get("rust-cov:rust-core-coverage");
    const install = cov.needs
      .map((n) => checks.get(n.includes(":") ? n : `rust-cov:${n}`))
      .find((c) => c.run === "pnpm install --frozen-lockfile");
    assert.ok(
      install,
      `${plan.profile}: rust-core-coverage needs a pnpm install`,
    );
    assert.equal(install.when, true, `${plan.profile}: that install runs`);
    // Exactly one install per profile: ex63 lanes share one checkout.
    const installs = [...checks.values()].filter(
      (c) => c.when && c.run === "pnpm install --frozen-lockfile",
    );
    assert.equal(installs.length, 1, plan.profile);
  }
  const hosted = buildPlan({ profile: "hosted", areas: coreOnly });
  const sub = selectLanes(hosted, ["rust-cov"]);
  assert.deepEqual(validatePlan(sub), []);
  assert.deepEqual(orderProblems(sub), []);
});

test("core doctests remain outside PR coverage; embed doctests and tui tests run on PRs", () => {
  for (const plan of plans()) {
    const cov = plan.lanes
      .find((l) => l.name === "rust-cov")
      .checks.find((c) => c.name === "rust-core-coverage");
    assert.equal(cov.env.OH_COV_DOCTESTS, "0");
    assert.equal(cov.env.OH_COV_TUI, "1");
    const runs = allRuns(plan).join("\n");
    assert.doesNotMatch(runs, /cargo test -p openhuman --doc/);
    assert.doesNotMatch(
      runs,
      /cargo test -p openhuman-(embed|tinyhumans)\b(?! --doc)/,
    );
    assert.match(runs, /cargo test -p openhuman-embed --doc/);
    const doc = plan.lanes
      .find((l) => l.name === "rust-lint")
      .checks.find((c) => c.name === "embed-rustdoc");
    assert.equal(doc.env.RUSTDOCFLAGS, "-D warnings");
    assert.match(doc.run, /cargo doc -p openhuman-embed --no-deps/);
    assert.match(runs, /node scripts\/run-embed-examples.mjs/);
    assert.doesNotMatch(runs, /-p openhuman --no-default-features$/m);
    assert.doesNotMatch(runs, /tool_output_tabulates_a_large_graph/);
  }
  assert.match(ciLanes, /scripts\/ci\/self-hosted\/lanes\.mjs/);
});

test("the complete suites run, not subsets", () => {
  const runs = allRuns(plans()[0]).join("\n");
  assert.match(runs, /pnpm --filter openhuman-app test:coverage(?! \S*\.test)/);
  assert.match(runs, /bash scripts\/ci\/rust-coverage\.sh/);
  assert.match(
    runs,
    /cargo llvm-cov nextest .*--manifest-path crates\/openhuman-app\/Cargo\.toml/,
  );
});

test("the lane plan retains the shared CI checks", () => {
  const shared = [
    "cargo fmt --manifest-path Cargo.toml --package openhuman --package openhuman-cli --package openhuman-embed --package openhuman-modules-linked --package openhuman-rpc --package openhuman-tinyhumans --package openhuman-tui -- --check",
    "node scripts/ci/check-openhuman-rust-layout.mjs",
    "node scripts/ci/check-storage-owner.mjs",
    "node scripts/ci/check-storage-schema.mjs",
    "pnpm agent:runtime-boundary",
    "pnpm rust:ignored-tests",
    "bash scripts/check-linux-tls-dependencies.sh",
    "bash scripts/ci/check-gated-test-allowlist.sh",
    "bash scripts/ci/orch-ip-gate.sh",
    "node scripts/ci/check-feature-forwarding.mjs",
    "node scripts/ci/check-crate-chain.mjs",
    "node scripts/ci/check-module-pins.mjs",
    "node scripts/ci/check-submodule-monotonic.mjs",
    "node scripts/ci/check-toolchain-image.mjs",
    "pnpm test:inventory",
    "pnpm --filter openhuman-app compile",
    "pnpm --filter openhuman-app format:check",
    "pnpm --filter openhuman-app lint",
    "pnpm i18n:check",
    "pnpm docs:test",
    "pnpm docs:check",
    "pnpm test:scripts",
    "cargo clippy -p openhuman-embed --all-targets -- -D warnings",
    "cargo check -p openhuman-embed --no-default-features",
    "cargo clippy -p openhuman-tinyhumans --all-targets -- -D warnings",
    "bash scripts/check-kernel-floor.sh --verbose",
    "bash scripts/ci/check-dep-sim-calibration.sh",
    "cargo clippy --manifest-path crates/openhuman-app/Cargo.toml -- -D warnings",
    "pwsh -NoProfile -File scripts/tests/OpenHumanWindowsInstall.Tests.ps1",
  ];
  const runs = allRuns(plans()[0]).join("\n");
  for (const cmd of shared) {
    assert.ok(runs.includes(cmd), `lane plan lost: ${cmd}`);
  }
});

test("untouched areas leave only the always-on gates", () => {
  const plan = buildPlan({ profile: "hosted", areas: NONE });
  const on = plan.lanes.flatMap((l) =>
    l.checks.filter((c) => c.when).map((c) => `${l.name}:${c.name}`),
  );
  assert.deepEqual(on, [
    "static:orch-ip-gate",
    "static:feature-forwarding",
    "static:crate-chain",
    "static:process-statics",
    "static:module-source",
    "static:storage-bypass",
    "static:module-pins",
    "static:submodule-monotonic",
  ]);
  assert.deepEqual(
    hostedMatrix(plan).map((g) => g.group),
    ["checks"],
  );
});

test("the storage area arms only the storage-drivers lane, with the sqlite and file drivers on", () => {
  for (const profile of ["hosted", "ex63"]) {
    const plan = buildPlan({
      profile,
      areas: { ...NONE, storage: true },
      env: profile === "ex63" ? EX63_ENV : {},
    });
    const on = plan.lanes.flatMap((l) =>
      l.checks.filter((c) => c.when).map((c) => `${l.name}:${c.name}`),
    );
    // Only the always-on gates plus this lane: the storage area arms no
    // frontend, Rust or other area lane.
    assert.deepEqual(
      on.filter((id) => !id.startsWith("static:")),
      [
        "storage-drivers:storage-e2e",
        "storage-drivers:storage-lib-tests",
        "storage-drivers:cron-module-tests",
        "storage-drivers:storage-session-store-tests",
      ],
      profile,
    );
    assert.deepEqual(
      on.filter((id) => id.startsWith("static:")),
      [
        "static:orch-ip-gate",
        "static:feature-forwarding",
        "static:crate-chain",
        "static:process-statics",
        "static:module-source",
        "static:storage-bypass",
        "static:module-pins",
        "static:submodule-monotonic",
      ],
      profile,
    );
    const lane = plan.lanes.find((l) => l.name === "storage-drivers");
    assert.equal(lane.active, true, profile);
    for (const check of lane.checks) {
      assert.match(
        check.run,
        /--features (?:session-store,)?storage-sqlite,storage-file(?:\s|$)/,
      );
    }
    // Every root storage target runs in the lane, so a new one cannot be
    // forgotten: the targets on disk are exactly the targets in the command.
    const e2e = lane.checks.find((c) => c.name === "storage-e2e").run;
    const onDisk = fs
      .readdirSync(path.join(repoRoot, "tests"))
      .filter((f) => /^(storage_.*_e2e|cli_storage_url_e2e)\.rs$/.test(f))
      .map((f) => f.replace(/\.rs$/, ""))
      .sort();
    const inLane = [...e2e.matchAll(/--test (\S+)/g)].map((m) => m[1]).sort();
    assert.deepEqual(inLane, onDisk, profile);
    // MongoDB runs the shared storage targets that support its feature set.
    // SQLite-only targets belong to the driver lane above, not MongoDB.
    const mongo = fs.readFileSync(
      path.join(repoRoot, ".github/workflows/storage-mongodb.yml"),
      "utf8",
    );
    const inMongo = [...mongo.matchAll(/--test (\S+)/g)]
      .map((m) => m[1])
      .sort();
    assert.deepEqual(
      inMongo,
      onDisk.filter((target) => target !== "storage_default_import_e2e"),
      "storage-mongodb.yml",
    );
    if (profile === "hosted") {
      assert.deepEqual(
        hostedMatrix(plan).map((g) => g.group),
        ["checks", "storage"],
      );
    }
  }
  // Without the area the lane stays off.
  const off = buildPlan({ profile: "hosted", areas: NONE });
  assert.equal(
    off.lanes.find((l) => l.name === "storage-drivers").active,
    false,
  );
});

test("hosted matrix only spins up groups with active lanes", () => {
  const docsOnly = hostedMatrix(
    buildPlan({ profile: "hosted", areas: { ...NONE, docs: true } }),
  );
  assert.deepEqual(
    docsOnly.map((g) => g.group),
    ["checks"],
  );
  const core = hostedMatrix(
    buildPlan({ profile: "hosted", areas: { ...NONE, rustCore: true } }),
  );
  assert.deepEqual(
    core.map((g) => g.group),
    ["checks", "rust-lint", "rust-cov"],
  );
  const pester = hostedMatrix(
    buildPlan({ profile: "hosted", areas: { ...NONE, installPs1: true } }),
  );
  assert.equal(pester.find((g) => g.group === "pester").container, "");
});

test("private lanes reuse checkout targets, preserve mold through RUSTFLAGS resets, and omit wrappers", () => {
  const plan = buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV });
  const rust = plan.lanes.filter((l) => l.heavy != null);
  assert.equal(
    new Set(rust.filter((l) => l.name !== "rust-cov").map((l) => l.targetDir))
      .size,
    1,
  );
  for (const lane of rust) {
    assert.match(lane.targetDir, /^target\/ci-private-/);
    assert.equal(
      lane.env.CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER,
      "/usr/local/bin/rust-mold-linker",
    );
    assert.equal(lane.env.RUSTC_WRAPPER, undefined);
    assert.equal(lane.env.RUSTC_WORKSPACE_WRAPPER, undefined);
  }
  assert.notEqual(
    rust.find((l) => l.name === "rust-cov").targetDir,
    rust.find((l) => l.name === "rust-lint").targetDir,
  );
  assert.ok(
    buildPlan({ profile: "hosted", areas: ALL }).lanes.every(
      (l) => !l.targetDir,
    ),
  );
});

test("private gates-off, storage, and Tauri execute equivalent coverage suites with nextest", () => {
  const plan = buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV });
  for (const name of ["rust-gates-off", "storage-drivers", "tauri"]) {
    for (const c of plan.lanes
      .find((l) => l.name === name)
      .checks.filter((c) => /cargo (test|llvm-cov)/.test(c.run))) {
      assert.match(c.run, /cargo llvm-cov nextest/);
      assert.match(c.run, /--no-tests=fail/);
      // Pinned cargo-llvm-cov 0.9.1 rejects combining these two flags;
      // --no-report already preserves previous instrumentation profiles.
      assert.doesNotMatch(c.run, /--no-clean(?:\s|$)/);
      assert.match(c.run, /--lcov --output-path ci-out\/lcov\//);
      assert.match(c.run, /--no-report .*\|\| rc=\$\?; cargo llvm-cov report/);
      assert.match(c.run, /exit "\$rc"$/);
    }
  }
  const checks = Object.fromEntries(
    plan.lanes.flatMap((l) => l.checks.map((c) => [c.name, c])),
  );
  assert.match(
    checks["gate-contract-tests-features"].run,
    /--features modules,mcp,e2e-test-support/,
  );
  assert.match(checks["gate-contract-tests-features"].run, /--test-threads=1/);
  assert.match(checks["cron-module-tests"].run, /--test-threads=1/);
  assert.equal(
    checks["storage-e2e"].env.NEXTEST_EXPECTED_TARGETS,
    "ci-out/nextest/storage-e2e-expected-targets.txt",
  );
  for (const target of [
    ...checks["storage-e2e"].run.matchAll(/--test (\S+)/g),
  ].map((match) => match[1])) {
    assert.ok(
      checks["storage-e2e"].run.includes(`'${target}'`),
      `expected inventory includes ${target}`,
    );
  }
  assert.match(
    checks["storage-lib-tests"].run,
    /not test\(cron::bus_host::tests::\)/,
  );
  assert.equal(
    checks["tauri-coverage"].env.CARGO_BUILD_TARGET_DIR,
    "target/ci-private-coverage",
  );
});

test("private auxiliary coverage reports after failed tests and preserves the failure", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lane-coverage-report-"));
  const cargo = path.join(dir, "cargo");
  const calls = path.join(dir, "calls");
  fs.writeFileSync(
    cargo,
    '#!/bin/sh\necho "$*" >> "$CARGO_CALLS"\nif [ "$1" = metadata ]; then echo \"{\\\"packages\\\":[]}\"; fi\nif [ "$2" = nextest ]; then exit 17; fi\nexit 0\n',
    { mode: 0o755 },
  );
  const plan = buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV });
  const check = plan.lanes.find((l) => l.name === "storage-drivers").checks[0];
  const result = spawnSync("bash", ["-o", "pipefail", "-c", check.run], {
    cwd: repoRoot,
    env: {
      ...process.env,
      PATH: `${dir}:${process.env.PATH}`,
      CARGO_CALLS: calls,
      NEXTEST_REPORT_DIR: path.join(dir, "reports"),
      NEXTEST_EXPECTED_TARGETS: path.join(dir, "expected-targets"),
    },
  });
  assert.equal(result.status, 17);
  assert.equal(
    fs
      .readFileSync(path.join(dir, "expected-targets"), "utf8")
      .trim()
      .split("\n").length,
    11,
  );
  const commands = fs.readFileSync(calls, "utf8").trim().split("\n");
  assert.equal(commands.length, 3);
  assert.match(commands[0], /^metadata /);
  assert.match(commands[1], /^llvm-cov nextest /);
  assert.match(commands[2], /^llvm-cov report /);
  const tauri = plan.lanes.find((l) => l.name === "tauri");
  assert.match(
    tauri.checks.find((c) => c.name === "build-tauri").run,
    /cargo build .*--bin OpenHuman/,
  );
  fs.rmSync(dir, { recursive: true, force: true });
});

test("private runner serializes independent lanes after failures even when parallelism is requested", async () => {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "lanes-private-"));
  fs.mkdirSync(path.join(out, "logs"));
  const check = (name, run) => ({ name, run, when: true, needs: [] });
  const plan = {
    profile: "ex63",
    lanes: [
      {
        name: "first",
        checks: [
          check("fails", "exit 1"),
          check("continues", "echo continued"),
        ],
      },
      { name: "second", checks: [check("independent", "echo independent")] },
    ],
  };
  const runner = new Runner(plan, { out, maxParallel: 8 });
  assert.equal(runner.maxParallel, 1);
  const lanes = await runner.run();
  assert.equal(lanes[0].checks[0].status, "failure");
  assert.equal(lanes[0].checks[1].status, "success");
  assert.equal(lanes[1].checks[0].status, "success");
  assert.ok(
    Date.parse(lanes[1].checks[0].start) >= Date.parse(lanes[0].checks[1].end),
  );
  assert.equal(
    new Runner({ ...plan, profile: "hosted" }, { out, maxParallel: 8 })
      .maxParallel,
    8,
  );
  fs.rmSync(out, { recursive: true, force: true });
});

test("area flags are read strictly from CI_AREA_*", () => {
  const areas = areasFromEnv({
    CI_AREA_RUST_CORE: "true",
    CI_AREA_FRONTEND: "false",
    CI_AREA_DOCS: "1",
  });
  assert.equal(areas.rustCore, true);
  assert.equal(areas.frontend, false);
  assert.equal(areas.docs, false);
});

test("argument parsing", () => {
  assert.deepEqual(
    parseArgs([
      "--profile",
      "hosted",
      "--lanes",
      "static, frontend",
      "--max-parallel",
      "2",
    ]).lanes,
    ["static", "frontend"],
  );
  assert.throws(() => parseArgs([]), /--profile/);
  assert.throws(
    () => parseArgs(["--profile", "ex63", "--bogus"]),
    /unknown argument/,
  );
});

function stubPlan() {
  const check = (name, run, extra = {}) => ({
    name,
    run,
    when: true,
    needs: [],
    ...extra,
  });
  return {
    profile: "hosted",
    lanes: [
      {
        name: "a",
        active: true,
        checks: [
          check("fails", "exit 3"),
          check("still-runs", "echo ran"),
          check("needs-failed", "echo never", { needs: ["fails"] }),
          check("report-only", "exit 1", { reportOnly: true }),
          check("off", "exit 1", { when: false }),
        ],
      },
      {
        name: "b",
        active: true,
        checks: [check("waits-on-a", "echo ok", { needs: ["a:still-runs"] })],
      },
    ],
  };
}

test("runner: a failure never stops later checks, blocks its dependants, and gates the run", async () => {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "lanes-test-"));
  fs.mkdirSync(path.join(out, "logs"));
  const lanes = await new Runner(stubPlan(), { out, maxParallel: 0 }).run();
  const status = Object.fromEntries(
    lanes.flatMap((l) =>
      l.checks.map((c) => [`${l.name}:${c.name}`, c.status]),
    ),
  );
  assert.deepEqual(status, {
    "a:fails": "failure",
    "a:still-runs": "success",
    "a:needs-failed": "blocked",
    "a:report-only": "failure",
    "a:off": "skipped",
    "b:waits-on-a": "success",
  });
  assert.deepEqual(gatingFailures({ lanes }), ["a:fails", "a:needs-failed"]);
  assert.match(fs.readFileSync(path.join(out, "logs", "a.log"), "utf8"), /ran/);
  const summary = renderSummary({ lanes });
  assert.match(
    summary,
    /\| a \| report-only \| failure \(report-only\) \| \S+ \|/,
  );
  assert.match(summary, /\| a \| needs-failed \| blocked \|/);
  fs.rmSync(out, { recursive: true, force: true });
});

test("compare-runs pairs the newest completed run per workflow by head SHA", async () => {
  const { pairRuns, laneMinutes, minutesBetween } =
    await import("../ci/self-hosted/compare-runs.mjs");
  const run = (sha, createdAt, updatedAt, status = "completed") => ({
    headSha: sha,
    createdAt,
    updatedAt,
    status,
    conclusion: "success",
  });
  const rows = pairRuns({
    lite: [run("a", "2026-09-01T00:00:00Z", "2026-09-01T01:30:00Z")],
    ex63: [
      run("a", "2026-09-01T00:00:00Z", "2026-09-01T00:40:00Z"),
      run("a", "2026-09-01T02:00:00Z", "2026-09-01T02:20:00Z"),
      run("a", "2026-09-01T03:00:00Z", "2026-09-01T03:05:00Z", "in_progress"),
    ],
    hosted: [run("b", "2026-09-01T00:00:00Z", "2026-09-01T00:10:00Z")],
  });
  assert.equal(rows.length, 1, "b has no CI Lite run, so it is not paired");
  assert.equal(
    minutesBetween(rows[0].ex63.createdAt, rows[0].ex63.updatedAt),
    20,
  );
  assert.deepEqual(
    laneMinutes({
      lanes: [
        {
          name: "rust-cov",
          checks: [
            { start: "2026-09-01T00:00:00Z", end: "2026-09-01T00:05:00Z" },
            { start: "2026-09-01T00:05:00Z", end: "2026-09-01T00:30:00Z" },
            { start: null, end: null },
          ],
        },
      ],
    }),
    { "rust-cov": 30 },
  );
});

test("step-per-lane mode: args, folded check groups and the lane table", () => {
  assert.equal(parseArgs(["--wait", "rust-cov"]).wait, "rust-cov");
  assert.equal(parseArgs(["--wait-all"]).waitAll, true);
  assert.equal(parseArgs(["--profile", "ex63", "--detach"]).detach, true);

  const state = { open: false };
  const out = [
    "[ci][lanes] ===== clippy =====",
    "$ cargo clippy",
    "warning: x",
    "[ci][lanes] clippy: failure (exit 101, 3s)",
    "[ci][lanes] next: blocked — needs lane:clippy",
    "[ci][lanes] ===== fmt =====",
    "[ci][lanes] ===== tests =====",
  ].map((l) => foldLine(l, state));
  assert.equal(out[0], "::group::clippy");
  assert.equal(
    out[3],
    "[ci][lanes] clippy: failure (exit 101, 3s)\n::endgroup::",
  );
  assert.equal(out[4], "[ci][lanes] next: blocked — needs lane:clippy");
  // A check whose outcome line never came is closed by the next check.
  assert.equal(out[6], "::endgroup::\n::group::tests");
  assert.equal(state.open, true);

  const table = renderLaneTable({
    name: "rust-lint",
    checks: [
      { name: "clippy", status: "failure", durationS: 3, reportOnly: false },
      { name: "off", status: "skipped", durationS: null, reportOnly: false },
      { name: "bench", status: "failure", durationS: 9, reportOnly: true },
    ],
  });
  assert.match(table, /failure\s+clippy 3s/);
  assert.doesNotMatch(table, /off/);
  assert.match(table, /bench 9s \(report-only\)/);
});

test("heavy-compile slots: priority order, FIFO within a priority, sized from RAM", async () => {
  const sem = new PrioritySemaphore(1);
  const order = [];
  await sem.acquire(5); // holder
  const waits = [
    sem.acquire(9).then(() => order.push("bench")),
    sem.acquire(1).then(() => order.push("lint")),
    sem.acquire(0).then(() => order.push("cov")),
    sem.acquire(1).then(() => order.push("gatesoff")),
  ];
  for (let i = 0; i < 4; i++) {
    sem.release();
    await new Promise((r) => setImmediate(r));
  }
  sem.release();
  await Promise.all(waits);
  assert.deepEqual(order, ["cov", "lint", "gatesoff", "bench"]);

  assert.equal(defaultHeavySlots(24 * 1024), 2);
  assert.equal(defaultHeavySlots(28 * 1024), 3);
  assert.equal(defaultHeavySlots(48 * 1024), 5);
  assert.equal(defaultHeavySlots(4 * 1024), 1);
});

test("runner: heavy lanes wait for a slot, light lanes never do", async () => {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "lanes-heavy-"));
  fs.mkdirSync(path.join(out, "logs"));
  const check = (name) => ({ name, run: "sleep 0.2", when: true, needs: [] });
  const plan = {
    profile: "hosted",
    lanes: [
      { name: "h1", heavy: 0, active: true, checks: [check("x")] },
      { name: "h2", heavy: 1, active: true, checks: [check("x")] },
      { name: "light", active: true, checks: [check("x")] },
    ],
  };
  const lanes = await new Runner(plan, {
    out,
    maxParallel: 0,
    maxHeavy: 1,
  }).run();
  const by = Object.fromEntries(lanes.map((l) => [l.name, l]));
  assert.equal(by.h1.heavyWaitS, 0);
  assert.ok(
    Date.parse(by.h2.checks[0].start) >= Date.parse(by.h1.checks[0].end),
  );
  assert.equal(by.light.heavyWaitS, null);
  assert.ok(
    Date.parse(by.light.checks[0].start) < Date.parse(by.h1.checks[0].end),
  );
  fs.rmSync(out, { recursive: true, force: true });
});

test("peak RSS follows the process tree, including setsid'd descendants", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oh-proc-"));
  const stat = (pid, ppid, pages) => {
    fs.mkdirSync(path.join(dir, String(pid)));
    // comm with a space and parens, as real process names can have.
    const fields = ["S", ppid, pid, ...Array(18).fill(0), pages];
    fs.writeFileSync(
      path.join(dir, String(pid), "stat"),
      `${pid} (cargo (x) y) ${fields.join(" ")}\n`,
    );
  };
  stat(100, 1, 256); // the check's bash: 1 MiB
  stat(101, 100, 512); // ci-cancel-aware.sh, own session after setsid: 2 MiB
  stat(102, 101, 1024 * 256); // rustc: 1 GiB
  stat(200, 1, 1024 * 256); // someone else's process
  fs.mkdirSync(path.join(dir, "self")); // non-numeric entries are skipped
  try {
    const table = processTable(dir);
    assert.equal(table.size, 4);
    assert.equal(table.get(102).ppid, 101);
    assert.equal(treeRssMiB(table, 100), 1 + 2 + 1024);
    assert.equal(treeRssMiB(table, 999), 0);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the rust-core path filter arms the lane for every crate the tui depends on", () => {
  const filter = fs.readFileSync(".github/ci-paths-filter.yml", "utf8");
  const block = filter.split(/^rust-tauri:/m)[0].split(/^rust-core:/m)[1];
  for (const crate of ["core", "embed", "tinyhumans", "rpc", "cli", "tui"]) {
    assert.ok(block.includes(`'crates/openhuman-${crate}/**'`), crate);
  }
});

test("module boundaries run for Rust and scripts changes in both profiles", () => {
  for (const profile of ["hosted", "ex63"]) {
    for (const area of ["rustCore", "rustTauri", "scripts"]) {
      const plan = buildPlan({
        profile,
        areas: { ...NONE, [area]: true },
        env: profile === "ex63" ? EX63_ENV : {},
      });
      const check = plan.lanes
        .flatMap((lane) => lane.checks)
        .find((check) => check.name === "module-boundaries");
      assert.equal(check.when, true, `${profile}:${area}`);
      assert.equal(
        check.run,
        "bash scripts/ci-cancel-aware.sh node scripts/ci/check-module-boundaries.mjs",
      );
    }
  }
});

test("module boundary policy and checker changes arm their own scripts lane", () => {
  const filters = fs.readFileSync(
    path.join(repoRoot, ".github/ci-paths-filter.yml"),
    "utf8",
  );
  const scripts = filters.split(/^scripts:\s*$/m)[1].split(/^\S[^\n]*:/m)[0];
  for (const file of [
    "scripts/ci/rust-coverage.sh",
    "scripts/ci/fetch-submodule-tags.sh",
  ]) {
    assert.ok(
      scripts.includes(`'${file}'`),
      `${file} must run its regressions`,
    );
  }
  for (const file of [
    "scripts/ci/check-module-boundaries.mjs",
    "scripts/ci/module-boundaries.json",
  ]) {
    assert.ok(
      scripts.includes(`'${file}'`),
      `${file} must arm the scripts area`,
    );
  }
});

test("process-static policy and checker changes arm their own scripts lane", () => {
  const filters = fs.readFileSync(
    path.join(repoRoot, ".github/ci-paths-filter.yml"),
    "utf8",
  );
  const scripts = filters.split(/^scripts:\s*$/m)[1].split(/^\S[^\n]*:/m)[0];
  for (const file of [
    "scripts/ci/rust-coverage.sh",
    "scripts/ci/fetch-submodule-tags.sh",
  ]) {
    assert.ok(
      scripts.includes(`'${file}'`),
      `${file} must run its regressions`,
    );
  }
  for (const file of [
    "scripts/ci/check-process-statics.mjs",
    "scripts/ci/check-crate-chain.mjs",
    "scripts/ci/process-statics.json",
  ]) {
    assert.ok(
      scripts.includes(`'${file}'`),
      `${file} must arm the scripts area`,
    );
  }
});

test("process-static scan uses cancellation supervision in both profiles", () => {
  for (const profile of ["hosted", "ex63"]) {
    const plan = buildPlan({
      profile,
      areas: NONE,
      env: profile === "ex63" ? EX63_ENV : {},
    });
    const check = plan.lanes
      .flatMap((lane) => lane.checks)
      .find((check) => check.name === "process-statics");
    assert.equal(
      check.run,
      "bash scripts/ci-cancel-aware.sh node scripts/ci/check-process-statics.mjs",
    );
  }
});

test("hosted containers repair the mounted home before module-dependent checks", () => {
  const hosted = ciLanes.slice(ciLanes.indexOf("  lanes:\n"));
  const repair = hosted.indexOf("      - name: Repair container home ownership");
  const dependencies = hosted.indexOf("      - name: Check out submodules");
  assert.ok(repair >= 0 && repair < dependencies);
  const step = hosted.slice(repair, hosted.indexOf("\n      - name:", repair + 1));
  assert.match(step, /if: matrix\.container != ''/);
  assert.match(step, /run: chown "\$\(id -u\):\$\(id -g\)" "\$HOME"/);
  assert.doesNotMatch(step, /chown\s+-R|chmod|TINYBUS|GITHUB_ENV/);
});

test("private frontend tools have an explicit 4 GiB Node heap while hosted keeps its default", () => {
  for (const plan of plans()) {
    const frontend = plan.lanes.find((lane) => lane.name === "frontend");
    if (plan.profile === "ex63") {
      assert.equal(frontend.env.NODE_OPTIONS, "--max-old-space-size=4096");
      const probe = spawnSync(
        process.execPath,
        [
          "-e",
          "console.log(require('node:v8').getHeapStatistics().heap_size_limit)",
        ],
        {
          env: { ...process.env, NODE_OPTIONS: frontend.env.NODE_OPTIONS },
          encoding: "utf8",
        },
      );
      assert.equal(probe.status, 0, probe.stderr);
      const limit = Number(probe.stdout.trim());
      assert.ok(
        limit >= 4096 * 1024 * 1024 && limit < 4352 * 1024 * 1024,
        `actual V8 heap limit ${limit}`,
      );
    } else {
      assert.equal(frontend.env.NODE_OPTIONS, undefined);
    }
  }
});

test("private CI executes the coverage-disabled Composio stack regression once without instrumentation", () => {
  for (const plan of plans()) {
    const lint = plan.lanes.find((lane) => lane.name === "rust-lint");
    const checks = lint.checks.filter(
      (check) => check.name === "composio-stack-regression-uninstrumented",
    );
    if (plan.profile === "ex63") {
      assert.equal(checks.length, 1);
      assert.equal(checks[0].when, true);
      assert.match(
        checks[0].run,
        /bash scripts\/ci\/nextest-coverage\.sh composio-stack-uninstrumented cargo nextest run --profile ci --no-tests=fail --no-fail-fast/,
      );
      assert.match(
        checks[0].run,
        /-p openhuman-cli --features "\$\(bash scripts\/ci\/product-features\.sh --core\)" --test composio_list_tools_stack_overflow_regression$/,
      );
      assert.doesNotMatch(checks[0].run, /llvm-cov|--no-default-features/);
      assert.equal(lint.targetDir, "target/ci-private-build");
    } else {
      assert.equal(checks.length, 0);
    }
  }
  const unrelated = buildPlan({ profile: "ex63", areas: NONE, env: EX63_ENV });
  assert.ok(
    unrelated.lanes
      .flatMap((lane) => lane.checks)
      .filter(
        (check) => check.name === "composio-stack-regression-uninstrumented",
      )
      .every((check) => !check.when),
  );
  const source = fs.readFileSync(
    path.join(
      repoRoot,
      "tests/composio_list_tools_stack_overflow_regression.rs",
    ),
    "utf8",
  );
  assert.match(source, /#!\[cfg\(not\(coverage\)\)\]/);
  assert.doesNotMatch(source, /^\s*#\[ignore/m);
});

test("nextest validation helpers arm Rust validation and script regressions on their own", () => {
  const filters = fs.readFileSync(
    path.join(repoRoot, ".github/ci-paths-filter.yml"),
    "utf8",
  );
  for (const area of ["rust-core", "rust-tauri", "storage", "scripts"]) {
    const block = filters
      .split(new RegExp(`^${area}:\\s*$`, "m"))[1]
      .split(/^\S[^\n]*:/m)[0];
    assert.ok(
      block.includes("'scripts/ci/self-hosted/**'"),
      `${area} covers guest verifier, linkage verifier and image pin changes`,
    );
    for (const file of [
      ".config/nextest.toml",
      "scripts/ci/nextest-coverage.sh",
      "scripts/ci/nextest-report.mjs",
    ]) {
      assert.ok(block.includes(`'${file}'`), `${file} must arm ${area}`);
    }
  }
  const scripts = filters.split(/^scripts:\s*$/m)[1].split(/^\S[^\n]*:/m)[0];
  for (const file of [
    "scripts/ci/rust-coverage.sh",
    "scripts/ci/fetch-submodule-tags.sh",
  ]) {
    assert.ok(
      scripts.includes(`'${file}'`),
      `${file} must run its regressions`,
    );
  }
  assert.ok(
    scripts.includes("'scripts/lib/**'"),
    "module-pin-inventory.mjs is covered by the shared script library glob",
  );
});

test("private coverage pools profiles for every Rust configuration without changing hosted or ordinary builds", () => {
  for (const plan of plans()) {
    const coverage = [];
    for (const lane of plan.lanes)
      for (const check of lane.checks) {
        const env = { ...lane.env, ...check.env };
        if (
          check.name === "rust-core-coverage" ||
          check.run.includes("cargo llvm-cov nextest")
        ) {
          coverage.push(check.name);
          assert.equal(
            env.LLVM_PROFILE_FILE_NAME,
            plan.profile === "ex63" ? "openhuman-%8m.profraw" : undefined,
            `${plan.profile}:${check.name}`,
          );
        } else if (
          /cargo (?:build|clippy|check|test|nextest run)/.test(check.run)
        ) {
          assert.equal(
            env.LLVM_PROFILE_FILE_NAME,
            undefined,
            `${plan.profile}:${check.name} is uninstrumented`,
          );
        }
      }
    if (plan.profile === "ex63") assert.equal(coverage.length, 8);
  }
});

test("private release cleanup follows successful examples and names only the ordinary release profile", () => {
  for (const plan of plans()) {
    const lint = plan.lanes.find((lane) => lane.name === "rust-lint");
    const cleanupIndex = lint.checks.findIndex(
      (check) => check.name === "release-example-cleanup",
    );
    if (plan.profile === "hosted") {
      assert.equal(cleanupIndex, -1);
      continue;
    }
    const examplesIndex = lint.checks.findIndex(
      (check) => check.name === "embed-offline-examples",
    );
    assert.equal(cleanupIndex, examplesIndex + 1);
    const cleanup = lint.checks[cleanupIndex];
    assert.equal(cleanup.when, true);
    assert.deepEqual(cleanup.needs, ["embed-offline-examples"]);
    assert.equal(
      cleanup.run,
      "bash scripts/ci-cancel-aware.sh cargo clean --profile release --target-dir target/ci-private-build",
    );
    assert.doesNotMatch(
      cleanup.run,
      /ci-private-coverage|tinymold|--profile dev|rm /,
    );
  }
});

test("failed offline examples block release cleanup while independent Rust validation continues", async () => {
  const plan = buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV });
  const lint = plan.lanes.find((lane) => lane.name === "rust-lint");
  const cleanup = lint.checks.find(
    (check) => check.name === "release-example-cleanup",
  );
  assert.ok(cleanup);
  const out = fs.mkdtempSync(
    path.join(os.tmpdir(), "release-cleanup-dependency-"),
  );
  try {
    fs.mkdirSync(path.join(out, "logs"));
    const checks = [
      { name: "embed-offline-examples", when: true, needs: [], run: "exit 1" },
      cleanup,
      { name: "independent", when: true, needs: [], run: "echo continued" },
    ];
    const result = await new Runner(
      { profile: "ex63", lanes: [{ name: "rust-lint", checks }] },
      { out, maxParallel: 1 },
    ).run();
    assert.deepEqual(
      result[0].checks.map((check) => check.status),
      ["failure", "blocked", "success"],
    );
  } finally {
    fs.rmSync(out, { recursive: true, force: true });
  }
});

test("private ordinary Rust builds keep line tables while coverage and hosted profiles remain unchanged", () => {
  for (const plan of plans()) {
    for (const lane of plan.lanes.filter((lane) => lane.heavy != null)) {
      const expected =
        lane.name === "rust-cov"
          ? "0"
          : plan.profile === "ex63"
            ? "line-tables-only"
            : undefined;
      assert.equal(
        lane.env.CARGO_PROFILE_DEV_DEBUG,
        expected,
        `${plan.profile}:${lane.name}`,
      );
      assert.equal(
        lane.env.CARGO_PROFILE_TEST_DEBUG,
        undefined,
        "test inherits the dev profile without a separate override",
      );
      assert.equal(
        lane.env.CARGO_PROFILE_RELEASE_DEBUG,
        undefined,
        "release keeps the manifest profile",
      );
      for (const check of lane.checks) {
        const env = { ...lane.env, ...check.env };
        const coverage =
          check.name === "rust-core-coverage" ||
          check.run.includes("cargo llvm-cov nextest") ||
          check.name === "tauri-coverage";
        if (coverage)
          assert.equal(
            env.CARGO_PROFILE_DEV_DEBUG,
            "0",
            `${plan.profile}:${check.name} keeps coverage debug=0`,
          );
        if (plan.profile === "ex63" && lane.name === "rust-lint")
          assert.equal(
            env.CARGO_PROFILE_DEV_DEBUG,
            "line-tables-only",
            check.name,
          );
        assert.equal(env.CARGO_PROFILE_TEST_DEBUG, undefined);
        assert.equal(env.CARGO_PROFILE_RELEASE_DEBUG, undefined);
      }
    }
  }
});

test("storage-bypass runs under cancellation supervision in both profiles", () => {
  for (const profile of ["hosted", "ex63"]) {
    const plan = buildPlan({
      profile,
      areas: NONE,
      env: profile === "ex63" ? EX63_ENV : {},
    });
    const check = plan.lanes
      .flatMap((lane) => lane.checks)
      .find((check) => check.name === "storage-bypass");
    assert.equal(check.when, true);
    assert.equal(
      check.run,
      "bash scripts/ci-cancel-aware.sh node scripts/ci/check-storage-bypass.mjs",
    );
  }
});

test("storage checker and debt policy changes arm their own scripts lane", () => {
  const filters = fs.readFileSync(
    path.join(repoRoot, ".github/ci-paths-filter.yml"),
    "utf8",
  );
  const scripts = filters.split(/^scripts:\s*$/m)[1].split(/^\S[^\n]*:/m)[0];
  for (const pattern of [
    "scripts/ci/check-storage-bypass.mjs",
    "scripts/ci/check-crate-chain.mjs",
    "scripts/ci/storage-bypass-debt.mjs",
    "scripts/ci/storage-bypass*.json",
    "scripts/ci/storage-bypass*.md",
  ])
    assert.ok(
      scripts.includes(`'${pattern}'`),
      `${pattern} must arm the scripts area`,
    );
});
