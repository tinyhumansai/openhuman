// Self-test for the CI lane flow: scripts/ci/self-hosted/{lanes-plan,lanes}.mjs
// (run by ci-fast.yml / ci-fast-hosted.yml through ci-lanes.yml).
import assert from "node:assert/strict";
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
  sccacheSummary,
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
const ciLite = fs.readFileSync(
  path.join(repoRoot, ".github", "workflows", "ci-lite.yml"),
  "utf8",
);

const ALL = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, true]));
const NONE = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, false]));
const EX63_ENV = { CI_SCRATCH_DIR: "/scratch" };

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

test("doctests, tui coverage and module-gated tests are left to pushes to main", () => {
  for (const plan of plans()) {
    const cov = plan.lanes
      .find((l) => l.name === "rust-cov")
      .checks.find((c) => c.name === "rust-core-coverage");
    assert.equal(cov.env.OH_COV_DOCTESTS, "0");
    assert.equal(cov.env.OH_COV_TUI, "0");
    const runs = allRuns(plan).join("\n");
    assert.doesNotMatch(runs, /cargo test -p openhuman --doc/);
    assert.doesNotMatch(runs, /cargo test -p openhuman-(embed|tinyhumans)\b/);
    assert.doesNotMatch(runs, /-p openhuman --no-default-features$/m);
    assert.doesNotMatch(runs, /tool_output_tabulates_a_large_graph/);
  }
  // ...where CI Lite still runs them.
  const lite = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/ci-lite.yml"),
    "utf8",
  );
  assert.match(lite, /on:\s*\n\s*push:\s*\n\s*branches: \[main\]/);
  assert.match(lite, /run: bash scripts\/ci\/run-module-gated-tests\.sh/);
  assert.match(lite, /run: bash scripts\/ci\/rust-coverage\.sh/);
  for (const cmd of [
    "cargo test -p openhuman-embed",
    "cargo test -p openhuman-tinyhumans",
    "cargo check --manifest-path Cargo.toml -p openhuman --no-default-features",
  ])
    assert.ok(lite.includes(cmd), `CI Lite no longer runs: ${cmd}`);
});

test("the complete suites run, not subsets", () => {
  const runs = allRuns(plans()[0]).join("\n");
  assert.match(runs, /pnpm --filter openhuman-app test:coverage(?! \S*\.test)/);
  assert.match(runs, /bash scripts\/ci\/rust-coverage\.sh/);
  assert.match(
    runs,
    /cargo llvm-cov --no-rustc-wrapper --manifest-path crates\/openhuman-app\/Cargo\.toml/,
  );
});

test("every ci-lite check the lanes claim to carry is still a ci-lite check", () => {
  // Commands shared verbatim with ci-lite.yml. If ci-lite changes one of these
  // the lane plan must move with it (and vice versa).
  const shared = [
    "cargo fmt --all -- --check",
    "node scripts/ci/check-openhuman-rust-layout.mjs",
    "pnpm agent:runtime-boundary",
    "pnpm rust:ignored-tests",
    "bash scripts/check-linux-tls-dependencies.sh",
    "bash scripts/ci/check-gated-test-allowlist.sh",
    "bash scripts/ci/orch-ip-gate.sh",
    "node scripts/ci/check-feature-forwarding.mjs",
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
    assert.ok(ciLite.includes(cmd), `ci-lite.yml no longer runs: ${cmd}`);
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
    "static:module-pins",
    "static:submodule-monotonic",
  ]);
  assert.deepEqual(
    hostedMatrix(plan).map((g) => g.group),
    ["checks"],
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

test("ex63 needs a scratch dir and gives every Rust lane its own target dir under it", () => {
  assert.throws(
    () => buildPlan({ profile: "ex63", areas: ALL, env: {} }),
    /CI_SCRATCH_DIR/,
  );
  const plan = buildPlan({ profile: "ex63", areas: ALL, env: EX63_ENV });
  const dirs = plan.lanes.map((l) => l.targetDir).filter(Boolean);
  assert.equal(new Set(dirs).size, dirs.length);
  for (const d of dirs) assert.match(d, /^\/scratch\/target\//);
  // hosted keeps cargo's default target dirs (restored by rust-cache).
  assert.ok(
    buildPlan({ profile: "hosted", areas: ALL }).lanes.every(
      (l) => !l.targetDir,
    ),
  );
});

test("instrumented coverage uses sccache on ex63 and no wrapper on hosted", () => {
  for (const plan of plans()) {
    const ex63 = plan.profile === "ex63";
    const cov = plan.lanes.find((l) => l.name === "rust-cov");
    assert.equal(cov.env.RUSTC_WRAPPER, ex63 ? "sccache" : undefined);
    const tauriCov = plan.lanes
      .find((l) => l.name === "tauri")
      .checks.find((c) => c.name === "tauri-coverage");
    assert.equal(tauriCov.env.RUSTC_WRAPPER, ex63 ? "sccache" : undefined);
    // The linker flag always goes: llvm-cov sets RUSTFLAGS itself.
    assert.match(
      tauriCov.run,
      ex63 ? /^unset RUSTFLAGS &&/ : /^unset RUSTFLAGS RUSTC_WRAPPER &&/,
    );
    assert.match(tauriCov.run, /--no-rustc-wrapper/);
  }
});

test("sccache summary reports hit rate and the store in use", () => {
  assert.equal(sccacheSummary(null), null);
  assert.equal(
    sccacheSummary({
      stats: {
        cache_hits: { counts: { Rust: 30 } },
        cache_misses: { counts: { Rust: 10 } },
      },
      cache_location: "webdav, name: , prefix: /",
    }),
    "sccache (shared store): 30 Rust hits, 10 misses (75%).",
  );
  assert.equal(
    sccacheSummary({
      stats: { cache_hits: { counts: {} }, cache_misses: { counts: {} } },
      cache_location: 'Local disk: "/cache/sccache"',
    }),
    "sccache (slot disk): 0 Rust hits, 0 misses.",
  );
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
    profile: "ex63",
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
