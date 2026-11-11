import assert from "node:assert/strict";
import { test } from "node:test";

import { SOURCES, decide, latestPrRun } from "../ci/ci-gate.mjs";

const [HOSTED, LITE] = SOURCES;

function entry(source, id, status, conclusion, job) {
  return {
    source,
    run: { id, status, conclusion, html_url: `https://runs/${id}` },
    job: job === undefined ? null : { ...job, html_url: `https://jobs/${id}` },
  };
}

test("only GitHub-hosted flows decide the gate", () => {
  assert.deepEqual(
    SOURCES.map((source) => source.workflow),
    ["ci-fast-hosted.yml", "ci-lite.yml"],
  );
});

test("the hosted lanes passing wins and cancels the other runs still going", () => {
  const verdict = decide([
    entry(HOSTED, 1, "completed", "success", {
      status: "completed",
      conclusion: "success",
    }),
    entry(LITE, 2, "in_progress", null, null),
  ]);
  assert.equal(verdict.state, "success");
  assert.equal(verdict.description, "CI Fast (hosted) passed");
  assert.equal(verdict.targetUrl, "https://jobs/1");
  assert.deepEqual(verdict.cancel, [{ id: 2, name: "CI Lite" }]);
});

test("a skipped decisive job is not a pass: the gate stays pending on the rest", () => {
  const verdict = decide([
    entry(HOSTED, 1, "completed", "success", {
      status: "completed",
      conclusion: "skipped",
    }),
    entry(LITE, 2, "queued", null, null),
  ]);
  assert.equal(verdict.state, "pending");
  assert.equal(verdict.description, "Waiting on CI Lite");
});

test("a hosted failure waits for CI Lite instead of failing the gate early", () => {
  const verdict = decide([
    entry(HOSTED, 1, "completed", "failure", {
      status: "completed",
      conclusion: "failure",
    }),
    entry(LITE, 2, "in_progress", null, null),
  ]);
  assert.equal(verdict.state, "pending");
});

test("everything finished with no pass fails and points at the failed job", () => {
  const verdict = decide([
    entry(HOSTED, 1, "completed", "failure", {
      status: "completed",
      conclusion: "failure",
    }),
    entry(LITE, 2, "completed", "failure", {
      status: "completed",
      conclusion: "failure",
    }),
  ]);
  assert.equal(verdict.state, "failure");
  assert.equal(
    verdict.description,
    "CI Fast (hosted): failure; CI Lite: failure",
  );
  assert.equal(verdict.targetUrl, "https://jobs/1");
});

test("finished flows whose decisive jobs never ran fail rather than pass", () => {
  const verdict = decide([
    entry(HOSTED, 1, "completed", "cancelled", null),
    entry(LITE, 2, "completed", "cancelled", {
      status: "completed",
      conclusion: "skipped",
    }),
  ]);
  assert.equal(verdict.state, "failure");
  assert.equal(verdict.description, "No CI flow ran its checks");
});

test("no run yet is pending", () => {
  assert.equal(decide([]).state, "pending");
});

test("latestPrRun ignores push runs and picks the newest PR run", () => {
  const run = latestPrRun([
    { id: 1, event: "pull_request", created_at: "2026-09-23T10:00:00Z" },
    { id: 2, event: "push", created_at: "2026-09-23T12:00:00Z" },
    { id: 3, event: "pull_request_target", created_at: "2026-09-23T11:00:00Z" },
  ]);
  assert.equal(run.id, 3);
  assert.equal(latestPrRun([{ id: 2, event: "push", created_at: "x" }]), null);
});
