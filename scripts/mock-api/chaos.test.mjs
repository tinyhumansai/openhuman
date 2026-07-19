import assert from "node:assert/strict";
import test from "node:test";

import {
  getMockServerPort,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from "./index.mjs";

// Chaos-toolkit coverage for the mock backend's httpFaultRules engine
// (plan.md §5.3). Beyond clean HTTP error statuses, authors need to reproduce
// two real outage shapes: a connection reset mid-response, and a 200 with a
// non-JSON body. These tests drive the real server over a socket.

function base() {
  const port = getMockServerPort();
  assert.ok(port, "mock server must be running");
  return `http://127.0.0.1:${port}`;
}

function setFaultRules(rules) {
  setMockBehavior("httpFaultRules", JSON.stringify(rules));
}

test.beforeEach(async () => {
  await stopMockServer();
  resetMockBehavior();
  await startMockServer();
});

test.afterEach(async () => {
  await stopMockServer();
});

test("mode:reset tears down the connection (client sees a network error)", async () => {
  setFaultRules([{ contains: "/chaos-reset", mode: "reset" }]);

  await assert.rejects(
    fetch(`${base()}/chaos-reset`),
    (err) => err instanceof TypeError || /reset|hang up|ECONNRESET|fetch failed/i.test(String(err)),
    "a reset rule must surface as a client-side network error, not a clean response",
  );
});

test("mode:malformed returns a 200 with a body that is not valid JSON", async () => {
  setFaultRules([{ contains: "/chaos-malformed", mode: "malformed" }]);

  const res = await fetch(`${base()}/chaos-malformed`);
  assert.equal(res.status, 200, "malformed mode defaults to a 200 status");

  const text = await res.text();
  assert.ok(text.length > 0, "malformed body must be non-empty");
  assert.throws(
    () => JSON.parse(text),
    "the malformed body must not parse as JSON (that is the whole point)",
  );
});

test("mode:malformed honours a custom status and body override", async () => {
  setFaultRules([
    { contains: "/chaos-custom", mode: "malformed", status: 502, body: "upstream said <html>" },
  ]);

  const res = await fetch(`${base()}/chaos-custom`);
  assert.equal(res.status, 502);
  assert.equal(await res.text(), "upstream said <html>");
});

test("default mode still injects a clean HTTP error status + JSON envelope", async () => {
  // Regression guard: adding chaos modes must not change the existing
  // status-injection behaviour when no mode is set.
  setFaultRules([{ contains: "/chaos-status", status: 503, error: "down for maintenance" }]);

  const res = await fetch(`${base()}/chaos-status`);
  assert.equal(res.status, 503);
  const parsed = await res.json();
  assert.equal(parsed.success, false);
  assert.equal(parsed.error, "down for maintenance");
});

test("a request that matches no fault rule is unaffected", async () => {
  setFaultRules([{ contains: "/chaos-reset", mode: "reset" }]);

  const res = await fetch(`${base()}/__admin/health`);
  assert.equal(res.status, 200, "unmatched requests must pass through to the real handler");
});
