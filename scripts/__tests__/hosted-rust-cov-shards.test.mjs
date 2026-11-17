// Hosted `rust-cov` runs as shards of one lane. Pins that the shards are the
// whole suite (every suite in exactly one shard), that the gate fails when a
// shard did not report, and that changed-line coverage still sees every lcov.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  AREA_ENV,
  HOSTED_GROUPS,
  RUST_COV_SHARDS,
  buildPlan,
  hostedMatrix,
} from "../ci/self-hosted/lanes-plan.mjs";
import { checkShards } from "../ci/self-hosted/merge-coverage-shards.mjs";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const runner = path.join(repoRoot, "scripts", "ci", "rust-coverage.sh");
const workflow = fs.readFileSync(
  path.join(repoRoot, ".github", "workflows", "ci-lanes.yml"),
  "utf8",
);
const ALL = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, true]));
const NONE = Object.fromEntries(Object.keys(AREA_ENV).map((k) => [k, false]));

test("hosted rust-cov is one group per shard, each running the rust-cov lane", () => {
  const groups = HOSTED_GROUPS.filter((g) => g.lanes.includes("rust-cov"));
  assert.deepEqual(
    groups.map((g) => g.shard),
    RUST_COV_SHARDS,
  );
  assert.deepEqual(
    groups.map((g) => g.group),
    RUST_COV_SHARDS.map((s) => `rust-cov-${s}`),
  );
  assert.equal(new Set(RUST_COV_SHARDS).size, RUST_COV_SHARDS.length);
  const matrix = hostedMatrix(
    buildPlan({ profile: "hosted", areas: { ...NONE, rustCore: true } }),
  );
  assert.deepEqual(
    matrix.filter((g) => g.lanes === "rust-cov").map((g) => g.shard),
    RUST_COV_SHARDS,
  );
  // A storage-only change arms the same lane, so it arms every shard.
  const storage = hostedMatrix(
    buildPlan({ profile: "hosted", areas: { ...NONE, storage: true } }),
  );
  assert.deepEqual(
    storage.filter((g) => g.lanes === "rust-cov").map((g) => g.shard),
    RUST_COV_SHARDS,
  );
  // No other group is a shard.
  assert.deepEqual(
    matrix.filter((g) => g.lanes !== "rust-cov").map((g) => g.shard),
    ["", ""],
  );
});

test("the workflow gives every shard its own runner, cache and artifact, and hands it its shard", () => {
  assert.match(workflow, /OH_COV_SHARD: \$\{\{ matrix\.shard \}\}/);
  // One Rust cache key per group: shards must not share or overwrite one.
  assert.match(workflow, /shared-key: ci-fast-\$\{\{ matrix\.group \}\}/);
  assert.match(workflow, /name: ci-out-\$\{\{ matrix\.group \}\}/);
  // Every rust-cov step is keyed on the shard groups, not the old single one.
  assert.doesNotMatch(workflow, /matrix\.group == 'rust-cov'/);
  assert.match(workflow, /startsWith\(matrix\.group, 'rust-cov-'\)/);
  // The shards run under nextest (one process per test, hung tests killed).
  assert.match(workflow, /tool: cargo-llvm-cov,cargo-nextest/);
});

test("rust-cov shards get a real sandbox jail; other groups keep the default container", () => {
  // Sandbox and shell tests plan onto TinyBox's jail, which needs bubblewrap
  // >= 0.8 and unprivileged user namespaces inside the container.
  const options = workflow.match(/^\s+options: (.+)$/m)?.[1] ?? "";
  for (const opt of [
    "seccomp=unconfined",
    "apparmor=unconfined",
    "systempaths=unconfined",
  ]) {
    assert.match(options, new RegExp(`startsWith\\(matrix\\.group, 'rust-cov-'\\) && '[^']*--security-opt ${opt}`));
  }
  assert.match(options, /\|\| '--init' \}\}$/, "non-rust-cov groups keep only --init");
  assert.doesNotMatch(options, /--privileged|--cap-add/);
  const step = workflow.match(
    /- name: Install bubblewrap for the sandbox jail\n(?:\s+#.*\n)*\s+if: (.+)\n\s+run: (.+)\n/,
  );
  assert.ok(step, "the bubblewrap install step exists");
  assert.match(step[1], /startsWith\(matrix\.group, 'rust-cov-'\)/);
  assert.equal(step[2].trim(), "bash scripts/ci/install-bubblewrap.sh");
  const installer = fs.readFileSync(
    path.join(repoRoot, "scripts", "ci", "install-bubblewrap.sh"),
    "utf8",
  );
  assert.match(installer, /^SHA256=[0-9a-f]{64}$/m, "the bubblewrap release is pinned by checksum");
  assert.match(installer, /sha256sum -c -/);
});

test("container lanes run with a private root-owned home before any step uses it", () => {
  // The runner's /github/home is owned by its uid while the container runs as
  // root, and the secrets driver refuses a namespace another user can mutate.
  const names = [...workflow.matchAll(/^\s+- name: (.+)$/gm)].map((m) => m[1]);
  const home = names.indexOf("Give the job a private home");
  assert.notEqual(home, -1, "the private-home step exists");
  for (const later of ["Trust the workspace for git", "Install bubblewrap for the sandbox jail", "Check out submodules"]) {
    assert.ok(names.indexOf(later) > home, `${later} runs after HOME is set`);
  }
  const step = workflow.match(
    /- name: Give the job a private home\n(?:\s+#.*\n)*\s+if: (.+)\n\s+run: \|\n((?:\s{10}.+\n)+)/,
  );
  assert.ok(step, "the private-home step has an if and a run block");
  assert.equal(step[1].trim(), "matrix.container != ''");
  assert.match(step[2], /install -d -o root -g root -m 0700 \/home\/ci/);
  assert.match(step[2], /echo "HOME=\/home\/ci" >> "\$GITHUB_ENV"/);
  // /root is an always-forbidden system root: a home there makes every
  // ~-relative default (e.g. ~/OpenHuman) a protected location.
  assert.doesNotMatch(step[2], /HOME=\/root/);
});

test("container lanes root-own every ancestor of the workspace before checkout", () => {
  // The runner creates /__w/<repo>/<repo> on the host as its own uid. Tests
  // keep scratch state under the checkout's target/, and the secrets driver
  // refuses a directory whose ancestors another user owns.
  const step = workflow.match(
    /- name: Give the job a private home\n(?:\s+#.*\n)*\s+if: (.+)\n\s+run: \|\n((?:\s{10}.+\n)+)/,
  );
  assert.ok(step, "the private-home step has an if and a run block");
  assert.match(step[2], /dir="\$GITHUB_WORKSPACE"/);
  assert.match(step[2], /while \[ "\$dir" != \/ \]; do/);
  assert.match(step[2], /chown root:root "\$dir"/);
  assert.match(step[2], /chmod go-w "\$dir"/);
  assert.match(step[2], /dir="\$\(dirname "\$dir"\)"/);
});

test("the gate refuses a shard that failed, was cancelled or never reported", () => {
  // Matrix results fold into needs.lanes.result: any failed shard makes it
  // "failure", any cancelled one "cancelled". Only success/skipped pass.
  assert.match(
    workflow,
    /case "\$\{\{ needs\.lanes\.result \}\}" in\n\s+success\|skipped\) ;;\n\s+\*\) echo "::error::A lane job did not pass/,
  );
  // A shard missing from the matrix, or one with no lcov, fails the gate.
  assert.match(workflow, /merge-coverage-shards\.mjs/);
  assert.match(workflow, /assert-coverage-presence\.sh .*--all/);
});

const lcov = (...files) => files.map((f) => `SF:${f}\nDA:1,1\nend_of_record\n`).join("");
const matrixOf = (groups) => ({
  include: groups.map((group) => ({ group, lanes: "rust-cov" })),
});
const everyGroup = RUST_COV_SHARDS.map((s) => `rust-cov-${s}`);

test("checkShards merges every shard's lcov", () => {
  const files = Object.fromEntries(
    everyGroup.map((g) => [g, lcov(`crates/${g}.rs`)]),
  );
  const result = checkShards({
    matrix: matrixOf(["checks", ...everyGroup]),
    read: (group) => files[group] ?? null,
  });
  assert.deepEqual(result.errors, []);
  for (const g of everyGroup) assert.match(result.merged, new RegExp(`crates/${g}.rs`));
});

test("checkShards fails on a missing shard, a missing report and an empty report", () => {
  const read = (group) => lcov(`${group}.rs`);
  const dropped = checkShards({
    matrix: matrixOf(everyGroup.slice(1)),
    read,
  });
  assert.equal(dropped.errors.length, 1);
  assert.match(dropped.errors[0], new RegExp(everyGroup[0]));
  assert.match(dropped.errors[0], /not in the job matrix/);

  const unreported = checkShards({
    matrix: matrixOf(everyGroup),
    read: (g) => (g === everyGroup[2] ? null : read(g)),
  });
  assert.equal(unreported.errors.length, 1);
  assert.match(unreported.errors[0], new RegExp(`${everyGroup[2]}.*no lcov`));

  const empty = checkShards({
    matrix: matrixOf(everyGroup),
    read: (g) => (g === everyGroup[3] ? "" : read(g)),
  });
  assert.equal(empty.errors.length, 1);
  assert.match(empty.errors[0], new RegExp(`${everyGroup[3]}.*no source records`));
});

test("checkShards is idle when the Rust core was not tested", () => {
  const result = checkShards({
    matrix: matrixOf(["checks"]),
    read: () => null,
    required: false,
  });
  assert.deepEqual(result.errors, []);
});

// ---- rust-coverage.sh: every suite lands in exactly one shard ----

function runShard(shard, { exitCode = 0, tui = "1" } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-cov-shard-"));
  try {
    for (const folder of ["bin", "scripts/ci", "crates/openhuman-cli", ".config", "tests"])
      fs.mkdirSync(path.join(dir, folder), { recursive: true });
    const write = (name, content) =>
      fs.writeFileSync(path.join(dir, name), content, { mode: 0o755 });
    for (const name of [
      "scripts/ci/nextest-coverage.sh",
      "scripts/ci/nextest-report.mjs",
      ".config/nextest.toml",
    ])
      fs.copyFileSync(path.join(repoRoot, name), path.join(dir, name));
    write("scripts/ci/product-features.sh", "#!/bin/bash\necho channels\n");
    write("scripts/ci-cancel-aware.sh", '#!/bin/bash\nexec "$@"\n');
    write(
      "scripts/ci/assert-coverage-presence.sh",
      '#!/bin/bash\necho presence >> "$MOCK_CALLS"\nexit 0\n',
    );
    write("crates/openhuman-cli/Cargo.toml", "");
    write("tests/active.rs", "#[test] fn active() {}\n");
    write("bin/rustup", "#!/bin/bash\nexit 1\n");
    write(
      "bin/cargo",
      `#!/bin/bash
printf '%s\\n' "$*" >> "$MOCK_CALLS"
[ "${exitCode}" = 0 ] || case "$*" in *"llvm-cov nextest"*) exit "${exitCode}" ;; esac
node - "$@" <<'JS'
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === 'metadata') console.log(JSON.stringify({ packages: [] }));
if (args[0] === 'llvm-cov' && args[1] === 'show-env') console.log('export CARGO_LLVM_COV_TARGET_DIR=target');
if (args[0] === 'nextest' && args[1] === 'list') {
  console.log(JSON.stringify({ 'rust-suites': { active: { 'binary-name': 'active', testcases: { active: { ignored: false, 'filter-match': { status: 'matches' } } } } } }));
}
const pos = args.indexOf('--config-file');
if (pos >= 0) {
  const config = fs.readFileSync(args[pos + 1], 'utf8');
  const report = JSON.parse(config.split('\\n').find(line => line.startsWith('path = ')).slice(7));
  fs.writeFileSync(report, '<testsuites><testsuite name="openhuman-cli::active" tests="1"></testsuite></testsuites>');
}
JS
`,
    );
    const env = {
      ...process.env,
      PATH: `${dir}/bin:${process.env.PATH}`,
      MOCK_CALLS: `${dir}/calls`,
      OH_COV_RUNNER: "nextest",
      OH_COV_DOCTESTS: "0",
      OH_COV_TUI: tui,
    };
    if (shard === undefined) delete env.OH_COV_SHARD;
    else env.OH_COV_SHARD = shard;
    const result = spawnSync("bash", [runner], { cwd: dir, env, encoding: "utf8" });
    const callsFile = `${dir}/calls`;
    return {
      status: result.status,
      stderr: result.stderr,
      calls: fs.existsSync(callsFile)
        ? fs.readFileSync(callsFile, "utf8").trim().split("\n")
        : [],
    };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// What a `cargo llvm-cov nextest` call runs: the package of each filter term
// (an -E filter) or of each -p, under its feature flags and target kinds.
function suitesOf(calls) {
  const out = [];
  for (const line of calls.filter((l) => l.includes("llvm-cov nextest"))) {
    const features = (line.match(/--features (\S+)/) ?? [])[1] ?? "";
    const filter = line.includes(" -E ") ? line.slice(line.indexOf(" -E ") + 4) : null;
    if (filter) {
      for (const term of filter.split(" | ")) {
        const pkg = term.match(/package\(=([^)]+)\)/)[1];
        out.push(`${pkg}${term.includes("kind(test)") ? " [integration]" : ""} +${features}`);
      }
    } else {
      const pkgs = [...line.matchAll(/-p (\S+)/g)].map((m) => m[1]);
      out.push(`${pkgs.join(",")} ${line.includes("--lib") ? "[lib]" : "[all]"} +${features}`);
    }
  }
  return out;
}

test("every rust-cov suite runs in exactly one shard, and the shards add up to the full run", () => {
  const full = runShard(undefined);
  assert.equal(full.status, 0, full.stderr);
  const expected = suitesOf(full.calls);
  assert.ok(expected.length >= 8, expected.join("\n"));

  const seen = [];
  for (const shard of RUST_COV_SHARDS) {
    const run = runShard(shard);
    assert.equal(run.status, 0, `${shard}: ${run.stderr}`);
    const suites = suitesOf(run.calls);
    assert.ok(suites.length > 0, `${shard} ran nothing`);
    seen.push(...suites);
    // Each shard writes its own report.
    assert.equal(
      run.calls.filter((l) => l.includes("llvm-cov report")).length,
      1,
      shard,
    );
  }
  // The core's unit tests are the one suite whose invocation differs between
  // the full run (inside the product graph) and the core shard (core alone).
  const norm = (list) =>
    list.map((s) => s.replace(/^openhuman \[lib\] \+/, "openhuman +")).sort();
  assert.equal(new Set(seen).size, seen.length, `a suite ran in two shards:\n${seen.join("\n")}`);
  const stripFeatures = (list) => norm(list).map((s) => s.replace(/ \+.*$/, ""));
  assert.deepEqual(stripFeatures(seen), stripFeatures(expected));
});

test("a shard defers the whole-tree presence check to the gate; the full run keeps it", () => {
  assert.ok(runShard(undefined).calls.includes("presence"));
  for (const shard of RUST_COV_SHARDS) {
    assert.ok(!runShard(shard).calls.includes("presence"), shard);
  }
});

test("a failing shard still reports its lcov, and fails", () => {
  for (const shard of RUST_COV_SHARDS) {
    const run = runShard(shard, { exitCode: 7 });
    assert.equal(run.status, 1, shard);
    assert.equal(run.calls.filter((l) => l.includes("llvm-cov report")).length, 1, shard);
  }
});

test("an unknown shard, or a shard on the cargo runner, is refused before anything builds", () => {
  const unknown = runShard("everything");
  assert.equal(unknown.status, 2);
  assert.deepEqual(unknown.calls, []);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rust-cov-cargo-"));
  try {
    const r = spawnSync("bash", [runner], {
      cwd: repoRoot,
      env: { ...process.env, OH_COV_SHARD: "core", OH_COV_RUNNER: "cargo" },
      encoding: "utf8",
    });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /needs OH_COV_RUNNER=nextest/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the tui section is skipped only in the tui shard when OH_COV_TUI=0", () => {
  assert.equal(suitesOf(runShard("tui", { tui: "0" }).calls).length, 0);
  assert.ok(suitesOf(runShard("facades", { tui: "0" }).calls).length > 0);
});
