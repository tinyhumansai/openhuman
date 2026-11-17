import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

test("local Linux E2E shards cover every CI suite", () => {
  const runner = fs.readFileSync(
    path.join(repoRoot, "app", "scripts", "e2e-run-shards.sh"),
    "utf8",
  );
  const workflow = fs.readFileSync(
    path.join(repoRoot, ".github", "workflows", "e2e-reusable.yml"),
    "utf8",
  );
  const flowRunner = fs.readFileSync(
    path.join(repoRoot, "app", "scripts", "e2e-run-all-flows.sh"),
    "utf8",
  );

  const runnerStart = runner.indexOf("SHARDS=(");
  const runnerEnd = runner.indexOf("\n)", runnerStart);
  assert.notEqual(runnerStart, -1, "local shard list is present");
  assert.notEqual(runnerEnd, -1, "local shard list is closed");
  const localShards = Array.from(
    runner.slice(runnerStart, runnerEnd).matchAll(/^\s*"([^:"]+):([^"]+)"$/gm),
    ([, name, suites]) => ({ name, suites }),
  );

  const ciShards = [];
  const allWorkflowShards = Array.from(
    workflow.matchAll(/^\s*- \{ name: ([\w-]+), suites: "([^"]+)" \}$/gm),
    ([, name, suites]) => ({ name, suites }),
  );
  const seen = new Set();
  for (const [, name, suites] of workflow.matchAll(
    /^\s*- \{ name: ([\w-]+), suites: "([^"]+)" \}$/gm,
  )) {
    if (!seen.has(name)) {
      seen.add(name);
      ciShards.push({ name, suites });
    }
  }

  assert.deepEqual(localShards, ciShards);
  const expectedShards = [
    { name: "foundation", suites: "auth,navigation,system" },
    { name: "chat", suites: "chat,skills,journeys" },
    { name: "connections", suites: "providers,provider-web,connectors" },
    { name: "settings", suites: "settings,notifications" },
  ];
  assert.deepEqual(localShards, expectedShards);
  assert.equal(
    allWorkflowShards.length,
    expectedShards.length * 3,
    "three OS matrices each have four groups",
  );
  for (
    let index = 0;
    index < allWorkflowShards.length;
    index += expectedShards.length
  ) {
    assert.deepEqual(
      allWorkflowShards.slice(index, index + expectedShards.length),
      expectedShards,
    );
  }

  const scheduled = Array.from(
    flowRunner.matchAll(
      /^\s*run "(test\/e2e\/specs\/[^\"]+)"\s+"[^\"]+"\s+"([^\"]+)"/gm,
    ),
    ([, spec, suite]) => ({ spec: path.join(repoRoot, "app", spec), suite }),
  );
  assert.ok(scheduled.length > 0);
  assert.equal(
    new Set(scheduled.map(({ spec }) => spec)).size,
    scheduled.length,
    "each native spec is scheduled once",
  );
  for (const { spec } of scheduled)
    assert.ok(fs.existsSync(spec), `scheduled native spec exists: ${spec}`);
  const suites = new Set(scheduled.map(({ suite }) => suite));
  const assignedSuites = new Set(
    localShards.flatMap(({ suites: list }) => list.split(",")),
  );
  assert.equal(
    suites.size,
    11,
    "the native runner schedules only its 11 nonempty suites",
  );
  assert.deepEqual([...suites].sort(), [...assignedSuites].sort());
  assert.ok(
    localShards.every(({ suites: list }) =>
      list.split(",").some((suite) => suites.has(suite)),
    ),
    "every OS shard group contains at least one nonempty native suite",
  );
});
