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

  const runnerStart = runner.indexOf("SHARDS=(");
  const runnerEnd = runner.indexOf("\n)", runnerStart);
  assert.notEqual(runnerStart, -1, "local shard list is present");
  assert.notEqual(runnerEnd, -1, "local shard list is closed");
  const localShards = Array.from(
    runner.slice(runnerStart, runnerEnd).matchAll(/^\s*"([^:"]+):([^"]+)"$/gm),
    ([, name, suites]) => ({ name, suites }),
  );

  const ciShards = [];
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
});
