import assert from "node:assert/strict";
import test from "node:test";

import {
  clearSocketEventLog,
  disconnectMockSockets,
  listSocketSessions,
  resetMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
} from "./index.mjs";
import { createSocket, onceSocket } from "./test-helpers/socket-client.mjs";

test.beforeEach(async () => {
  await stopMockServer();
  resetMockBehavior();
  clearSocketEventLog();
});

test.afterEach(async () => {
  disconnectMockSockets();
  await stopMockServer();
});

test("rejects connections that omit a required token", async () => {
  const started = await startMockServer(18575, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  setMockBehaviors({ socketAuthMode: "required" }, "replace");
  const rejectedSocket = createSocket(baseUrl, {
    auth: {},
    transports: ["polling"],
    upgrade: false,
  });

  try {
    const error = await onceSocket(rejectedSocket, "connect_error");
    assert.match(String(error?.message || error), /No token provided/);
    assert.equal(listSocketSessions().length, 0);
  } finally {
    rejectedSocket.disconnect();
  }
});
