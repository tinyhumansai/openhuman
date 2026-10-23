import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  classifySpecFiles,
  verifyDiagnosticBytes,
} from "../../app/scripts/e2e-web-selection.mjs";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const script = path.join(repoRoot, "app/scripts/e2e-web-shards.sh");
const selectionResolver = path.join(
  repoRoot,
  "app/scripts/e2e-web-selection.mjs",
);
fs.mkdirSync(path.join(repoRoot, "target"), { recursive: true });
const fixtureRoot = fs.mkdtempSync(
  path.join(repoRoot, "target", "e2e-web-shards-test-"),
);

function run(args, env = {}) {
  return spawnSync("bash", [script, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

test("four lanes assign each of 64 shards and nine special units once", () => {
  const units = [];
  const schedulerSource = fs.readFileSync(script, "utf8");
  assert.match(
    schedulerSource,
    /--grep-invert='Connections \(aliases\|deep links\)\|Skills explorer'/,
  );
  assert.doesNotMatch(schedulerSource, /Chat Harness - Subagent/);
  for (let lane = 1; lane <= 4; lane += 1) {
    const result = run([`--lane=${lane}`, "--lanes=4", "--list"]);
    assert.equal(result.status, 0, result.stderr);
    units.push(...JSON.parse(result.stdout));
  }

  const shards = units.filter((unit) => unit.kind === "shard");
  const specials = units.filter((unit) => unit.kind !== "shard");
  assert.equal(shards.length, 64);
  assert.deepEqual(
    shards
      .map((unit) => unit.shard)
      .sort((a, b) => Number.parseInt(a) - Number.parseInt(b)),
    Array.from({ length: 64 }, (_, index) => `${index + 1}/64`),
  );
  assert.equal(specials.length, 9);
  assert.deepEqual(
    specials.map((unit) => unit.index).sort((a, b) => a - b),
    Array.from({ length: 9 }, (_, index) => index),
  );
  assert.equal(
    new Set(units.map((unit) => `${unit.kind}:${unit.index}`)).size,
    73,
  );
});

test("invalid lane is rejected", () => {
  const result = run(["--lane=5", "--lanes=4", "--list"]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /exceeds lane count/);
});

test("strict and diagnostic selections partition discovered specs fail-closed", () => {
  const requiredResult = spawnSync(
    "node",
    [selectionResolver, "--selection=required", "--json"],
    {
      cwd: repoRoot,
      encoding: "utf8",
    },
  );
  assert.equal(requiredResult.status, 0, requiredResult.stderr);
  const required = JSON.parse(requiredResult.stdout);

  const diagnosticResult = spawnSync(
    "node",
    [selectionResolver, "--selection=diagnostic", "--json"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  assert.equal(diagnosticResult.status, 0, diagnosticResult.stderr);
  const diagnostic = JSON.parse(diagnosticResult.stdout);

  const allResult = spawnSync(
    "node",
    [selectionResolver, "--selection=all", "--json"],
    {
      cwd: repoRoot,
      encoding: "utf8",
    },
  );
  assert.equal(allResult.status, 0, allResult.stderr);
  const all = JSON.parse(allResult.stdout);

  assert.equal(diagnostic.files.length, 1);
  assert.match(diagnostic.files[0], /aui-tool-result-elements\.spec\.ts$/);
  assert.ok(
    required.files.includes("test/playwright/specs/agent-review.spec.ts"),
  );
  assert.ok(
    required.files.includes(
      "test/playwright/specs/command-palette-keyboard.spec.ts",
    ),
  );
  assert.ok(
    required.files.includes(
      "test/playwright/specs/skills-search-install.spec.ts",
    ),
  );
  assert.ok(
    required.files.includes(
      "test/playwright/specs/connections-tab-deeplinks.spec.ts",
    ),
  );
  assert.ok(
    required.files.includes(
      "test/playwright/specs/webhooks-tunnel-flow.spec.ts",
    ),
  );
  assert.deepEqual(
    [...required.files, ...diagnostic.files].sort(),
    all.files,
    "required and diagnostic must be disjoint and cover all discovered specs",
  );
  assert.equal(
    new Set([...required.files, ...diagnostic.files]).size,
    all.files.length,
  );
});

test("selection discovery works in the CI image without ripgrep", () => {
  const emptyPath = fs.mkdtempSync(path.join(fixtureRoot, "path-without-rg-"));
  const selections = ["all", "required", "diagnostic"].map((selection) => {
    const result = spawnSync(
      process.execPath,
      [selectionResolver, `--selection=${selection}`, "--json"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: { ...process.env, PATH: emptyPath },
      },
    );
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout);
  });

  assert.deepEqual(
    selections.map((result) => [result.selection, result.files.length]),
    [
      ["all", 111],
      ["required", 110],
      ["diagnostic", 1],
    ],
  );
});

test("selection classifier rejects unknown diagnostics and protected-owner demotion", () => {
  const base = {
    protectedOwnerFiles: ["test/playwright/specs/owner.spec.ts"],
    diagnostics: [{ path: "test/playwright/specs/legacy.spec.ts" }],
  };
  assert.throws(
    () => classifySpecFiles(["test/playwright/specs/owner.spec.ts"], base),
    /diagnostic inventory differs from the reviewed file allowlist/,
  );
  assert.throws(
    () =>
      classifySpecFiles(
        [
          "test/playwright/specs/owner.spec.ts",
          "test/playwright/specs/aui-tool-result-elements.spec.ts",
        ],
        {
          protectedOwnerFiles: [
            "test/playwright/specs/owner.spec.ts",
            "test/playwright/specs/aui-tool-result-elements.spec.ts",
          ],
          diagnostics: [
            { path: "test/playwright/specs/aui-tool-result-elements.spec.ts" },
          ],
        },
      ),
    /protected browser owners cannot be diagnostic/,
  );
});

test("diagnostic source bytes are pinned to their reviewed baseline", () => {
  assert.throws(
    () =>
      verifyDiagnosticBytes(
        {
          diagnostics: [
            {
              path: "test/playwright/specs/legacy.spec.ts",
              sha256: "0".repeat(64),
            },
          ],
        },
        () => Buffer.from("changed bytes"),
      ),
    /diagnostic source changed/,
  );
});

test("CI Full requires every Playwright lane and manual workflow shares the artifact scheduler", () => {
  const full = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/ci-full.yml"),
    "utf8",
  );
  const manual = fs.readFileSync(
    path.join(repoRoot, ".github/workflows/e2e-playwright.yml"),
    "utf8",
  );
  const job = full.slice(
    full.indexOf("  playwright-e2e:"),
    full.indexOf(
      "  # Runs only the exact unchanged AUI",
      full.indexOf("  playwright-e2e:"),
    ),
  );
  assert.match(job, /fail-fast:\s*false/);
  assert.match(job, /lane:\s*\[1, 2, 3, 4\]/);
  assert.doesNotMatch(job, /continue-on-error:/);
  assert.match(full, /- playwright-e2e/);
  assert.match(
    full,
    /\["Playwright E2E"\]="\$\{\{ needs\['playwright-e2e'\]\.result \}\}"/,
  );
  assert.match(manual, /name: Build Playwright E2E artifact/);
  assert.match(manual, /name: e2e-playwright-linux-\$\{\{ github\.run_id \}\}/);
  assert.match(manual, /lane:\s*\[1, 2, 3, 4\]/);
  assert.match(
    manual,
    /e2e-web-shards\.sh --lane=\$\{\{ matrix\.lane \}\} --lanes=4 --selection=all/,
  );
  const diagnosticJob = full.slice(
    full.indexOf("  playwright-legacy-diagnostic:"),
    full.indexOf(
      "  # Full desktop E2E",
      full.indexOf("  playwright-legacy-diagnostic:"),
    ),
  );
  assert.match(diagnosticJob, /continue-on-error:\s*true/);
  assert.match(diagnosticJob, /--selection=diagnostic/);
  assert.doesNotMatch(
    full
      .match(/needs:\s*\[[^\]]*playwright-legacy-diagnostic[^\]]*\]/g)
      ?.join("\n") ?? "",
    /./,
  );
});

test("required selection forwards every healthy spec and never the diagnostic file", () => {
  const runner = path.join(fixtureRoot, "selection-runner.sh");
  fs.writeFileSync(
    runner,
    '#!/usr/bin/env bash\nprintf "%s|%s\\n" "$OPENHUMAN_WORKSPACE" "$*" >> "$CALL_LOG"\n',
  );
  fs.chmodSync(runner, 0o755);
  const callLog = path.join(fixtureRoot, "required-calls.log");
  const workspace = path.join(fixtureRoot, "required-workspaces");
  const result = run(["--lane=4", "--lanes=4", "--selection=required"], {
    E2E_WEB_SESSION_RUNNER: `bash ${runner}`,
    CALL_LOG: callLog,
    OPENHUMAN_WORKSPACE: workspace,
  });

  assert.equal(result.status, 0, result.stderr);
  const calls = fs.readFileSync(callLog, "utf8").trim().split("\n");
  assert.equal(calls.length, 18);
  const joined = calls.join("\n");
  assert.doesNotMatch(joined, /aui-tool-result-elements\.spec\.ts/);
  for (const required of [
    "agent-review.spec.ts",
    "auth-access-control.spec.ts",
    "command-palette-keyboard.spec.ts",
    "skills-search-install.spec.ts",
    "webhooks-tunnel-flow.spec.ts",
  ]) {
    assert.match(joined, new RegExp(required.replaceAll(".", "\\.")));
  }
  assert.ok(
    calls.every((call) => call.includes("/selection-required/lane-4/")),
  );
});

test("diagnostic selection is one file-atomic unit and propagates failure", () => {
  const runner = path.join(fixtureRoot, "diagnostic-runner.sh");
  fs.writeFileSync(
    runner,
    '#!/usr/bin/env bash\nprintf "%s|%s\\n" "$OPENHUMAN_WORKSPACE" "$*" >> "$CALL_LOG"\nexit 1\n',
  );
  fs.chmodSync(runner, 0o755);
  const callLog = path.join(fixtureRoot, "diagnostic-calls.log");
  const workspace = path.join(fixtureRoot, "diagnostic-workspaces");
  const result = run(["--lane=1", "--lanes=1", "--selection=diagnostic"], {
    E2E_WEB_SESSION_RUNNER: `bash ${runner}`,
    CALL_LOG: callLog,
    OPENHUMAN_WORKSPACE: workspace,
  });

  assert.equal(result.status, 1, result.stderr);
  const calls = fs.readFileSync(callLog, "utf8").trim().split("\n");
  assert.equal(calls.length, 1);
  assert.match(calls[0], /aui-tool-result-elements\.spec\.ts/);
  assert.match(calls[0], /selection-diagnostic\/lane-1\/diagnostic-0/);
  assert.equal(
    fs
      .readFileSync(
        path.join(
          workspace,
          "selection-diagnostic/lane-1/receipts/diagnostic-0.exit",
        ),
        "utf8",
      )
      .trim(),
    "1",
  );
});

test("diagnostic lane with no assigned file is rejected before launching a runner", () => {
  const callLog = path.join(fixtureRoot, "empty-diagnostic-calls.log");
  const result = run(["--lane=2", "--lanes=4", "--selection=diagnostic"], {
    CALL_LOG: callLog,
  });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /assigns no units/);
  assert.equal(fs.existsSync(callLog), false);
});

test("failure artifact names are unique across all four Playwright lanes", () => {
  const workflows = [
    [
      "CI Full",
      fs.readFileSync(
        path.join(repoRoot, ".github/workflows/ci-full.yml"),
        "utf8",
      ),
    ],
    [
      "manual Playwright",
      fs.readFileSync(
        path.join(repoRoot, ".github/workflows/e2e-playwright.yml"),
        "utf8",
      ),
    ],
  ];

  for (const [workflow, source] of workflows) {
    const matches = [
      ...source.matchAll(
        /name: (e2e-playwright-[^\r\n]*failure-logs-[^\r\n]+)/g,
      ),
    ];
    assert.ok(matches.length, `${workflow} must upload its failure artifact`);
    const laneNames = matches.flatMap((match) =>
      Array.from({ length: 4 }, (_, index) =>
        match[1].replace(/\$\{\{\s*matrix\.lane\s*\}\}/g, String(index + 1)),
      ),
    );
    assert.equal(
      new Set(laneNames).size,
      laneNames.length,
      `${workflow} artifact names must be unique across jobs and lanes`,
    );
  }
});

test("failed unit traces survive when later Playwright units run", () => {
  const runner = path.join(fixtureRoot, "clearing-runner.sh");
  fs.writeFileSync(
    runner,
    `#!/usr/bin/env bash
set -euo pipefail
output=""
for arg in "$@"; do
  case "$arg" in --output=*) output="${"${arg#*=}"}" ;; esac
done
if [[ -z "$output" ]]; then output="$SHARED_OUTPUT"; fi
rm -rf "$output"
mkdir -p "$output"
printf '%s|%s\\n' "$OPENHUMAN_WORKSPACE" "$output" >> "$CALL_LOG"
count=0
[[ ! -f "$CALL_COUNT" ]] || count="$(cat "$CALL_COUNT")"
count=$((count + 1))
printf '%s\\n' "$count" > "$CALL_COUNT"
if (( count == 1 )); then
  printf 'first-unit-trace\\n' > "$output/trace.zip"
  exit 1
fi
`,
  );
  fs.chmodSync(runner, 0o755);
  const callLog = path.join(fixtureRoot, "trace-calls.log");
  const callCount = path.join(fixtureRoot, "trace-call-count");
  const sharedOutput = path.join(fixtureRoot, "shared-test-results");
  const workspace = path.join(fixtureRoot, "trace-workspaces");

  const result = run(["--lane=4", "--lanes=4"], {
    E2E_WEB_SESSION_RUNNER: `bash ${runner}`,
    CALL_LOG: callLog,
    CALL_COUNT: callCount,
    SHARED_OUTPUT: sharedOutput,
    OPENHUMAN_WORKSPACE: workspace,
  });

  assert.equal(result.status, 1, result.stderr);
  const calls = fs.readFileSync(callLog, "utf8").trim().split("\n");
  assert.equal(
    calls.length,
    18,
    "later assigned units still run after failure",
  );
  const [, firstOutput] = calls[0].split("|");
  const [, secondOutput] = calls[1].split("|");
  assert.ok(path.isAbsolute(firstOutput));
  assert.ok(fs.existsSync(path.join(firstOutput, "trace.zip")));
  assert.notEqual(firstOutput, secondOutput);
  assert.equal(
    fs
      .readFileSync(
        path.join(workspace, "selection-all/lane-4/receipts/special-3.exit"),
        "utf8",
      )
      .trim(),
    "1",
  );
  assert.equal(
    fs
      .readFileSync(
        path.join(workspace, "selection-all/lane-4/receipts/special-7.exit"),
        "utf8",
      )
      .trim(),
    "0",
  );
});

test("failed units retain failure while later units run in distinct workspaces", () => {
  const runner = path.join(fixtureRoot, "runner.sh");
  fs.writeFileSync(
    runner,
    '#!/usr/bin/env bash\nprintf "%s|%s\\n" "$OPENHUMAN_WORKSPACE" "$*" >> "$CALL_LOG"\n[[ "$*" != *"--shard=4/64"* ]]\n',
  );
  fs.chmodSync(runner, 0o755);
  const callLog = path.join(fixtureRoot, "calls.log");
  const workspace = path.join(fixtureRoot, "workspaces");
  const result = run(["--lane=4", "--lanes=4"], {
    E2E_WEB_SESSION_RUNNER: `bash ${runner}`,
    CALL_LOG: callLog,
    OPENHUMAN_WORKSPACE: workspace,
  });
  assert.equal(result.status, 1, result.stderr);
  const calls = fs.readFileSync(callLog, "utf8").trim().split("\n");
  assert.equal(calls.length, 18);
  assert.ok(calls.some((call) => call.includes("--shard=4/64")));
  assert.ok(calls.some((call) => call.includes("--shard=64/64")));
  const paths = calls.map((call) => call.split("|")[0]);
  assert.equal(new Set(paths).size, paths.length);
  assert.equal(
    fs
      .readFileSync(
        path.join(workspace, "selection-all/lane-4/receipts/shard-4.exit"),
        "utf8",
      )
      .trim(),
    "1",
  );
  assert.equal(
    fs
      .readFileSync(
        path.join(workspace, "selection-all/lane-4/receipts/shard-64.exit"),
        "utf8",
      )
      .trim(),
    "0",
  );
});

test.after(() => fs.rmSync(fixtureRoot, { recursive: true, force: true }));
