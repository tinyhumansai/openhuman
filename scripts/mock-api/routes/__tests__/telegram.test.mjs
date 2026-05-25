/**
 * Unit tests for the mock Telegram Bot API route handler.
 *
 * Pattern: construct a minimal `ctx` object (matching what server.mjs
 * provides), call the handler, and assert on the captured response.
 *
 * Run via:
 *   node --test scripts/mock-api/routes/__tests__/telegram.test.mjs
 * or through the project test runner:
 *   pnpm debug unit scripts/mock-api/routes/__tests__/telegram.test.mjs
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  resetMockBehavior,
  resetMockTelegram,
  setMockBehavior,
  pushMockTelegramUpdate,
  getMockTelegramSent,
  startMockServer,
  stopMockServer,
} from "../../index.mjs";
import { handleTelegram } from "../telegram.mjs";

// ── Helpers ────────────────────────────────────────────────────────────────

function createRes() {
  return {
    statusCode: 0,
    headers: {},
    body: "",
    writeHead(status, headers = {}) {
      this.statusCode = status;
      this.headers = { ...this.headers, ...headers };
    },
    setHeader(name, value) {
      this.headers[name] = value;
    },
    end(chunk = "") {
      this.body += String(chunk);
    },
    json() {
      return JSON.parse(this.body);
    },
  };
}

function makeCtx(method, path, body = null) {
  return {
    method,
    url: path,
    body: body ? JSON.stringify(body) : "",
    parsedBody: body,
    res: createRes(),
  };
}

// ── Setup / teardown ───────────────────────────────────────────────────────

test.beforeEach(() => {
  resetMockBehavior();
  resetMockTelegram();
});

// ── getMe ──────────────────────────────────────────────────────────────────

test("getMe returns default bot info", async () => {
  const ctx = makeCtx("POST", "/bot12345:TOKEN/getMe");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
  assert.equal(payload.result.is_bot, true);
  assert.equal(payload.result.username, "e2e_test_bot");
  assert.equal(payload.result.id, 123456789);
});

test("getMe returns custom username from behavior", async () => {
  setMockBehavior("telegramBotUsername", "my_custom_bot");
  const ctx = makeCtx("GET", "/bot12345:TOKEN/getMe");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  const payload = ctx.res.json();
  assert.equal(payload.result.username, "my_custom_bot");
});

test("getMe returns 401 when telegramGetMeFails=1", async () => {
  setMockBehavior("telegramGetMeFails", "1");
  const ctx = makeCtx("POST", "/botANYTOKEN/getMe");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 401);
  const payload = ctx.res.json();
  assert.equal(payload.ok, false);
  assert.equal(payload.error_code, 401);
});

// ── getUpdates ─────────────────────────────────────────────────────────────

test("getUpdates returns empty array when queue is empty", async () => {
  setMockBehavior("telegramPollDelayMs", "0");
  const ctx = makeCtx("POST", "/botTOKEN/getUpdates");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
  assert.deepEqual(payload.result, []);
});

test("getUpdates returns injected update", async () => {
  setMockBehavior("telegramPollDelayMs", "0");
  const update = {
    update_id: 1,
    message: {
      message_id: 1,
      from: { id: 9999, first_name: "Alice" },
      chat: { id: 9999, type: "private" },
      text: "hello bot",
    },
  };
  pushMockTelegramUpdate(update);

  const ctx = makeCtx("POST", "/botTOKEN/getUpdates");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  const payload = ctx.res.json();
  assert.equal(payload.result.length, 1);
  assert.deepEqual(payload.result[0], update);
});

test("getUpdates drains queue on each call", async () => {
  setMockBehavior("telegramPollDelayMs", "0");
  pushMockTelegramUpdate({ update_id: 1, message: { text: "first" } });
  pushMockTelegramUpdate({ update_id: 2, message: { text: "second" } });

  // First call — should return both
  const ctx1 = makeCtx("POST", "/botTOKEN/getUpdates");
  await handleTelegram(ctx1);
  const payload1 = ctx1.res.json();
  assert.equal(payload1.result.length, 2);

  // Second call — queue is now empty
  const ctx2 = makeCtx("POST", "/botTOKEN/getUpdates");
  await handleTelegram(ctx2);
  const payload2 = ctx2.res.json();
  assert.deepEqual(payload2.result, []);
});

// ── sendMessage ────────────────────────────────────────────────────────────

test("sendMessage records message and returns proper shape", async () => {
  const ctx = makeCtx("POST", "/botTOKEN/sendMessage", {
    chat_id: 42,
    text: "Hello from test!",
    parse_mode: "Markdown",
  });
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
  assert.equal(typeof payload.result.message_id, "number");
  assert.equal(payload.result.chat.id, 42);
  assert.equal(payload.result.text, "Hello from test!");

  // Verify it was recorded
  const sent = getMockTelegramSent();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].method, "sendMessage");
  assert.equal(sent[0].body.text, "Hello from test!");
  assert.equal(sent[0].message_id, payload.result.message_id);
});

test("sendMessage returns 400 when telegramSendFails=1", async () => {
  setMockBehavior("telegramSendFails", "1");
  const ctx = makeCtx("POST", "/botTOKEN/sendMessage", {
    chat_id: 42,
    text: "will fail",
  });
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 400);
  const payload = ctx.res.json();
  assert.equal(payload.ok, false);
  assert.equal(payload.error_code, 400);
});

// ── Simple record methods ──────────────────────────────────────────────────

test("sendChatAction records and returns ok", async () => {
  const ctx = makeCtx("POST", "/botTOKEN/sendChatAction", {
    chat_id: 42,
    action: "typing",
  });
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
  assert.equal(payload.result, true);
});

test("deleteWebhook returns ok", async () => {
  const ctx = makeCtx("POST", "/botTOKEN/deleteWebhook");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
});

// ── Media send methods ─────────────────────────────────────────────────────

test("sendPhoto records and returns message_id", async () => {
  const ctx = makeCtx("POST", "/botTOKEN/sendPhoto", { chat_id: 99 });
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
  assert.equal(typeof payload.result.message_id, "number");

  const sent = getMockTelegramSent();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].method, "sendPhoto");
});

// ── Token validation ───────────────────────────────────────────────────────

test("rejects mismatched token when telegramBotToken is set", async () => {
  setMockBehavior("telegramBotToken", "correctToken");
  const ctx = makeCtx("POST", "/botwrongToken/getMe");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 401);
  const payload = ctx.res.json();
  assert.equal(payload.ok, false);
});

test("accepts correct token when telegramBotToken is set", async () => {
  setMockBehavior("telegramBotToken", "correctToken");
  const ctx = makeCtx("POST", "/botcorrectToken/getMe");
  const handled = await handleTelegram(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const payload = ctx.res.json();
  assert.equal(payload.ok, true);
});

// ── Non-Telegram paths ─────────────────────────────────────────────────────

test("returns false for non-bot paths", async () => {
  const ctx = makeCtx("GET", "/api/something");
  const handled = await handleTelegram(ctx);
  assert.equal(handled, false);
});

// ── Admin round-trip (stateful, uses full HTTP server) ─────────────────────

test("admin inject-update + sent-list round-trip", async () => {
  const { port } = await startMockServer(18562, { retryIfInUse: true });
  const base = `http://127.0.0.1:${port}`;

  try {
    // Inject a single update
    const injectSingle = await fetch(`${base}/__admin/telegram/inject-update`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        update_id: 100,
        message: { text: "hello", chat: { id: 1 } },
      }),
    });
    assert.equal(injectSingle.status, 200);
    const injectBody = await injectSingle.json();
    assert.equal(injectBody.ok, true);
    assert.equal(injectBody.queued, 1);

    // Drain via getUpdates
    const updatesRes = await fetch(`${base}/botTEST123/getUpdates`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ timeout: 0 }),
    });
    const updatesBody = await updatesRes.json();
    assert.equal(updatesBody.ok, true);
    assert.equal(updatesBody.result.length, 1);
    assert.equal(updatesBody.result[0].update_id, 100);

    // Send a message
    await fetch(`${base}/botTEST123/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: 1, text: "reply" }),
    });

    // Check sent list
    const sentRes = await fetch(`${base}/__admin/telegram/sent`);
    const sentBody = await sentRes.json();
    assert.equal(sentBody.ok, true);
    assert.equal(sentBody.messages.length, 1);
    assert.equal(sentBody.messages[0].method, "sendMessage");
    assert.equal(sentBody.messages[0].body.text, "reply");
  } finally {
    await stopMockServer();
  }
});

test("admin inject-update accepts { updates: [...] } batch form", async () => {
  const { port } = await startMockServer(18563, { retryIfInUse: true });
  const base = `http://127.0.0.1:${port}`;

  try {
    const injectBatch = await fetch(`${base}/__admin/telegram/inject-update`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        updates: [
          { update_id: 200, message: { text: "msg1" } },
          { update_id: 201, message: { text: "msg2" } },
        ],
      }),
    });
    const batchBody = await injectBatch.json();
    assert.equal(batchBody.queued, 2);

    // Drain
    const updatesRes = await fetch(`${base}/botX/getUpdates`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    const updatesBody = await updatesRes.json();
    assert.equal(updatesBody.result.length, 2);
  } finally {
    await stopMockServer();
  }
});

test("admin telegram reset clears state", async () => {
  const { port } = await startMockServer(18564, { retryIfInUse: true });
  const base = `http://127.0.0.1:${port}`;

  try {
    // Inject an update and a sent message
    await fetch(`${base}/__admin/telegram/inject-update`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ update_id: 999, message: { text: "pre-reset" } }),
    });
    await fetch(`${base}/botX/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: 1, text: "pre-reset" }),
    });

    // Reset telegram state only
    const resetRes = await fetch(`${base}/__admin/telegram/reset`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
    const resetBody = await resetRes.json();
    assert.equal(resetBody.ok, true);

    // Sent should be empty now
    const sentRes = await fetch(`${base}/__admin/telegram/sent`);
    const sentBody = await sentRes.json();
    assert.deepEqual(sentBody.messages, []);

    // Queue should be empty (getUpdates returns [])
    const updatesRes = await fetch(`${base}/botX/getUpdates`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    const updatesBody = await updatesRes.json();
    assert.deepEqual(updatesBody.result, []);
  } finally {
    await stopMockServer();
  }
});

test("global admin reset also clears telegram state", async () => {
  const { port } = await startMockServer(18565, { retryIfInUse: true });
  const base = `http://127.0.0.1:${port}`;

  try {
    await fetch(`${base}/botX/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: 1, text: "before global reset" }),
    });

    // Global reset
    await fetch(`${base}/__admin/reset`, { method: "POST" });

    const sentRes = await fetch(`${base}/__admin/telegram/sent`);
    const sentBody = await sentRes.json();
    assert.deepEqual(sentBody.messages, []);
  } finally {
    await stopMockServer();
  }
});
