import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, ".."); // scripts/agent-batch
const VALIDATE = join(ROOT, "validate.mjs");
const OVERLAP = join(ROOT, "overlap.mjs");
const STATUS = join(ROOT, "status.mjs");
const EXAMPLE = resolve(
  ROOT,
  "..",
  "..",
  "docs",
  "agent-workflows",
  "pilot-batch-example.json",
);

function writeTempSpec(spec) {
  const dir = mkdtempSync(join(tmpdir(), "agent-batch-test-"));
  const path = join(dir, "spec.json");
  writeFileSync(path, JSON.stringify(spec, null, 2));
  return path;
}

function run(script, args, opts = {}) {
  return spawnSync(process.execPath, [script, ...args], {
    encoding: "utf8",
    ...opts,
  });
}

test("validate.mjs exits 0 on the canonical example spec", () => {
  const r = run(VALIDATE, [EXAMPLE]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.match(r.stdout, /ok: batch example-pilot-2026-05-15/);
});

test("validate.mjs exits 1 on a spec with malformed branch", () => {
  const path = writeTempSpec({
    batch_id: "pilot-test",
    base_repo: "tinyhumansai/openhuman",
    base_branch: "main",
    tracking_issue: 1480,
    agents: [
      {
        id: "a01",
        issue: 100,
        title: "t",
        branch: "feature/foo",
        owned_paths: ["src/openhuman/foo/"],
      },
    ],
  });
  const r = run(VALIDATE, [path]);
  assert.strictEqual(r.status, 1, r.stdout);
  assert.match(r.stderr, /branch must match cursor/);
});

test("overlap.mjs exits 0 on disjoint example", () => {
  const r = run(OVERLAP, [EXAMPLE]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.match(r.stdout, /disjoint paths/);
});

test("overlap.mjs exits 1 when two agents own the same prefix", () => {
  const path = writeTempSpec({
    batch_id: "pilot-test",
    base_repo: "tinyhumansai/openhuman",
    base_branch: "main",
    tracking_issue: 1480,
    agents: [
      {
        id: "a01",
        issue: 100,
        title: "t",
        branch: "cursor/a01-100-x",
        owned_paths: ["src/openhuman/foo/"],
      },
      {
        id: "a02",
        issue: 101,
        title: "t",
        branch: "cursor/a02-101-y",
        owned_paths: ["src/openhuman/foo/inner/"],
      },
    ],
  });
  const r = run(OVERLAP, [path]);
  assert.strictEqual(r.status, 1, r.stdout);
  assert.match(r.stderr, /ownership collision/);
  assert.match(r.stderr, /a01 ↔ a02/);
});

test("status.mjs renders a markdown table from a fixture (no gh needed)", () => {
  const fixture = [
    {
      headRefName: "cursor/a01-9001-memory-namespace-logging",
      number: 5001,
      url: "https://github.com/tinyhumansai/openhuman/pull/5001",
      state: "OPEN",
      statusCheckRollup: "SUCCESS",
    },
    {
      headRefName: "cursor/a02-9002-cron-rpc-dedupe",
      number: 5002,
      url: "https://github.com/tinyhumansai/openhuman/pull/5002",
      state: "OPEN",
      statusCheckRollup: "FAILURE",
    },
  ];
  const dir = mkdtempSync(join(tmpdir(), "agent-batch-test-"));
  const fixturePath = join(dir, "fixture.json");
  writeFileSync(fixturePath, JSON.stringify(fixture));
  const r = run(STATUS, [EXAMPLE, "--fixture", fixturePath]);
  assert.strictEqual(r.status, 0, r.stderr);
  assert.match(r.stdout, /<!-- batch:example-pilot-2026-05-15 -->/);
  assert.match(r.stdout, /\| a01 \|.*\[#5001\]/);
  assert.match(r.stdout, /\| a02 \|.*failing/);
  // a03 has no PR in the fixture — must still appear with placeholders.
  assert.match(r.stdout, /\| a03 \|.*\| — \| — \| no pr \|/i);
});
