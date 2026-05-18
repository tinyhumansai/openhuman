import assert from "node:assert/strict";
import test from "node:test";

import {
  resetMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from "../../index.mjs";

test.beforeEach(async () => {
  await stopMockServer();
  resetMockBehavior();
});

test.afterEach(async () => {
  await stopMockServer();
});

test("persists created conversations across requests", async () => {
  const started = await startMockServer(18575, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  const createdResponse = await fetch(`${baseUrl}/conversations`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ title: "Fixture conversation", channel: "web" }),
  });
  assert.equal(createdResponse.status, 200);
  const createdBody = await createdResponse.json();
  const createdId = createdBody?.data?.id;
  assert.equal(typeof createdId, "string");

  const listResponse = await fetch(`${baseUrl}/conversations`);
  const listBody = await listResponse.json();
  assert.equal(Array.isArray(listBody?.data), true);
  assert.equal(
    listBody.data.some(
      (entry) =>
        entry.id === createdId && entry.title === "Fixture conversation",
    ),
    true,
  );

  const detailResponse = await fetch(`${baseUrl}/conversations/${createdId}`);
  const detailBody = await detailResponse.json();
  assert.equal(detailBody?.data?.id, createdId);
  assert.equal(Array.isArray(detailBody?.data?.messages), true);
});

test("does not reseed conversations after deleting everything until reset", async () => {
  const started = await startMockServer(18577, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  const initialList = await fetch(`${baseUrl}/conversations`);
  const initialBody = await initialList.json();
  const ids = Array.isArray(initialBody?.data)
    ? initialBody.data.map((entry) => entry.id)
    : [];

  for (const id of ids) {
    await fetch(`${baseUrl}/conversations/${id}`, { method: "DELETE" });
  }

  const afterDelete = await fetch(`${baseUrl}/conversations`);
  const afterDeleteBody = await afterDelete.json();
  assert.deepEqual(afterDeleteBody?.data, []);
});

test("keeps server-controlled ids on create and patch routes", async () => {
  const started = await startMockServer(18578, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  const createdConversation = await fetch(`${baseUrl}/conversations`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id: "client-conv", title: "Client title" }),
  }).then((response) => response.json());
  assert.notEqual(createdConversation?.data?.id, "client-conv");

  const createdCron = await fetch(`${baseUrl}/settings/cron-jobs`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id: "client-cron", name: "Client cron" }),
  }).then((response) => response.json());
  const cronId = createdCron?.data?.id;
  assert.notEqual(cronId, "client-cron");

  await fetch(`${baseUrl}/settings/cron-jobs/${cronId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id: "mutated-cron", enabled: false }),
  });

  const cronList = await fetch(`${baseUrl}/settings/cron-jobs`).then(
    (response) => response.json(),
  );
  const patchedCron = cronList.data.find((entry) => entry.id === cronId);
  assert.equal(patchedCron.id, cronId);
  assert.equal(patchedCron.enabled, false);
});

test("applies injected HTTP fault rules without route-specific controller logic", async () => {
  const started = await startMockServer(18576, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  setMockBehaviors(
    {
      httpFaultRules: JSON.stringify([
        {
          method: "GET",
          pathRegex: "^/auth/me(?:\\?.*)?$",
          status: 503,
          body: { success: false, error: "Synthetic outage" },
        },
      ]),
    },
    "replace",
  );

  const response = await fetch(`${baseUrl}/auth/me`);
  assert.equal(response.status, 503);
  const body = await response.json();
  assert.equal(body.error, "Synthetic outage");
});
