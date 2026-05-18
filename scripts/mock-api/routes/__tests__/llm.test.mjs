import test from "node:test";
import assert from "node:assert/strict";

import { handleLlmCompletions } from "../llm.mjs";
import { handleIntegrations } from "../integrations.mjs";
import {
  listMockLlmThreads,
  resetMockBehavior,
  resetMockLlmThreads,
  setMockBehaviors,
} from "../../state.mjs";

function createMockResponse() {
  return {
    headers: {},
    statusCode: null,
    body: "",
    chunks: [],
    ended: false,
    setHeader(name, value) {
      this.headers[name] = value;
    },
    writeHead(status, headers = {}) {
      this.statusCode = status;
      Object.assign(this.headers, headers);
    },
    write(chunk) {
      const text = String(chunk);
      this.chunks.push(text);
      this.body += text;
    },
    end(chunk = "") {
      if (chunk) this.write(chunk);
      this.ended = true;
    },
  };
}

function makeCtx({
  method = "POST",
  url = "/chat/completions",
  parsedBody = {
    model: "gpt-oss",
    messages: [{ role: "user", content: "hello" }],
  },
  headers = {},
} = {}) {
  return {
    method,
    url,
    parsedBody,
    req: { headers },
    res: createMockResponse(),
  };
}

test.beforeEach(() => {
  resetMockBehavior();
  resetMockLlmThreads();
});

test("handles root chat completions path with default fallback", () => {
  const ctx = makeCtx({ url: "/chat/completions" });

  const handled = handleLlmCompletions(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const body = JSON.parse(ctx.res.body);
  assert.equal(body.model, "gpt-oss");
  assert.equal(body.choices[0].message.content, "Hello from e2e mock agent");
});

test("matches request rules against path and authorization header", () => {
  setMockBehaviors(
    {
      llmRequestRules: JSON.stringify([
        {
          path: "/v1/chat/completions",
          model: "gpt-4.1-mini",
          authorization: "Bearer sk-test",
          content: "matched via request rule",
        },
      ]),
    },
    "replace",
  );

  const ctx = makeCtx({
    url: "/v1/chat/completions",
    parsedBody: {
      model: "gpt-4.1-mini",
      messages: [{ role: "user", content: "hello" }],
    },
    headers: { authorization: "Bearer sk-test" },
  });

  const handled = handleLlmCompletions(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 200);
  const body = JSON.parse(ctx.res.body);
  assert.equal(body.choices[0].message.content, "matched via request rule");
});

test("streams request-rule scripts for root chat completions path", async () => {
  setMockBehaviors(
    {
      llmRequestRules: JSON.stringify([
        {
          path: "/chat/completions",
          stream: true,
          streamScript: [{ text: "hello" }, { finish: "stop" }],
        },
      ]),
    },
    "replace",
  );

  const ctx = makeCtx({
    url: "/chat/completions",
    parsedBody: {
      model: "gpt-oss",
      stream: true,
      messages: [{ role: "user", content: "stream please" }],
    },
  });

  const handled = handleLlmCompletions(ctx);
  assert.equal(handled, true);

  await new Promise((resolve) => setTimeout(resolve, 80));

  assert.equal(ctx.res.statusCode, 200);
  assert.match(ctx.res.body, /data: .*hello/);
  assert.match(ctx.res.body, /data: \[DONE\]/);
  assert.equal(ctx.res.ended, true);
});

test("returns HTTP error for streaming rules with status >= 400", () => {
  setMockBehaviors(
    {
      llmRequestRules: JSON.stringify([
        {
          path: "/chat/completions",
          stream: true,
          status: 401,
          error: "unauthorized",
          type: "auth_error",
        },
      ]),
    },
    "replace",
  );

  const ctx = makeCtx({
    url: "/chat/completions",
    parsedBody: {
      model: "gpt-oss",
      stream: true,
      messages: [{ role: "user", content: "stream please" }],
    },
  });

  const handled = handleLlmCompletions(ctx);

  assert.equal(handled, true);
  assert.equal(ctx.res.statusCode, 401);
  assert.equal(ctx.res.headers["Content-Type"], "application/json");
  const body = JSON.parse(ctx.res.body);
  assert.equal(body.error.message, "unauthorized");
  assert.equal(body.error.type, "auth_error");
  assert.doesNotMatch(ctx.res.body, /^data:/m);
});

test("returns false for non-LLM routes", () => {
  const ctx = makeCtx({ method: "GET", url: "/chat/completions" });
  assert.equal(handleLlmCompletions(ctx), false);
});

test("streams reasoning deltas for reasoning-family models", async () => {
  const ctx = makeCtx({
    url: "/chat/completions",
    parsedBody: {
      model: "openhuman-reasoning-mock",
      stream: true,
      messages: [{ role: "user", content: "compare these rollout options" }],
    },
  });

  assert.equal(handleLlmCompletions(ctx), true);
  await new Promise((resolve) => setTimeout(resolve, 220));

  assert.match(ctx.res.body, /reasoning_content/);
  assert.match(ctx.res.body, /Recommendation:/);
  assert.match(ctx.res.body, /data: \[DONE\]/);
});

test("returns tool calls for agentic models and resolves follow-up turns", () => {
  const first = makeCtx({
    parsedBody: {
      model: "openhuman-agentic-mock",
      mockThreadId: "agent-thread-1",
      messages: [
        { role: "user", content: "search the release notes and report back" },
      ],
      tools: [{ type: "function", function: { name: "web_search" } }],
    },
  });

  assert.equal(handleLlmCompletions(first), true);
  const firstBody = JSON.parse(first.res.body);
  assert.equal(firstBody.choices[0].finish_reason, "tool_calls");
  assert.equal(
    firstBody.choices[0].message.tool_calls[0].function.name,
    "web_search",
  );

  const second = makeCtx({
    parsedBody: {
      model: "openhuman-agentic-mock",
      mockThreadId: "agent-thread-1",
      messages: [
        { role: "user", content: "search the release notes and report back" },
        {
          role: "tool",
          content:
            "Release notes mention Socket.IO support and dynamic mock routes.",
        },
        { role: "user", content: "okay now summarize the result" },
      ],
    },
  });

  assert.equal(handleLlmCompletions(second), true);
  const secondBody = JSON.parse(second.res.body);
  assert.match(
    secondBody.choices[0].message.content,
    /Socket\.IO support and dynamic mock routes/i,
  );
});

test("updates coding responses across turns with thread memory", () => {
  const first = makeCtx({
    parsedBody: {
      model: "gpt-5-codex-mock",
      mockThreadId: "code-thread-1",
      messages: [{ role: "user", content: "write a tiny typescript helper" }],
    },
  });

  assert.equal(handleLlmCompletions(first), true);
  const firstBody = JSON.parse(first.res.body);
  assert.match(firstBody.choices[0].message.content, /```ts/);

  const second = makeCtx({
    parsedBody: {
      model: "gpt-5-codex-mock",
      mockThreadId: "code-thread-1",
      messages: [
        { role: "user", content: "make it async and keep it in typescript" },
      ],
    },
  });

  assert.equal(handleLlmCompletions(second), true);
  const secondBody = JSON.parse(second.res.body);
  assert.match(secondBody.choices[0].message.content, /Updated TS version/i);
  assert.match(secondBody.choices[0].message.content, /async function runTask/);
});

test("shortens summarization responses across turns", () => {
  const first = makeCtx({
    parsedBody: {
      model: "openhuman-summary-mock",
      mockThreadId: "summary-thread-1",
      messages: [
        {
          role: "user",
          content:
            "Summarize this: the mock backend now supports stateful routes, socket sessions, fault injection, and more realistic provider flows.",
        },
      ],
    },
  });

  assert.equal(handleLlmCompletions(first), true);
  const firstBody = JSON.parse(first.res.body);

  const second = makeCtx({
    parsedBody: {
      model: "openhuman-summary-mock",
      mockThreadId: "summary-thread-1",
      messages: [{ role: "user", content: "shorter" }],
    },
  });

  assert.equal(handleLlmCompletions(second), true);
  const secondBody = JSON.parse(second.res.body);
  assert.ok(
    secondBody.choices[0].message.content.length <=
      firstBody.choices[0].message.content.length,
  );
});

test("lists multiple mock model families from the integrations catalog", () => {
  const ctx = {
    method: "GET",
    url: "/openai/v1/models",
    parsedBody: null,
    res: createMockResponse(),
  };

  assert.equal(handleIntegrations(ctx), true);
  assert.equal(ctx.res.statusCode, 200);
  const body = JSON.parse(ctx.res.body);
  const ids = body.data.map((item) => item.id);
  assert.ok(ids.includes("openhuman-reasoning-mock"));
  assert.ok(ids.includes("openhuman-agentic-mock"));
  assert.ok(ids.includes("gpt-5-codex-mock"));
  assert.ok(ids.includes("openhuman-summary-mock"));
});

test("records thread state for multi-turn mock LLM sessions", () => {
  const ctx = makeCtx({
    parsedBody: {
      model: "openhuman-summary-mock",
      mockThreadId: "thread-state-1",
      messages: [
        { role: "user", content: "summarize the latest provider status" },
      ],
    },
  });

  assert.equal(handleLlmCompletions(ctx), true);
  const threads = listMockLlmThreads();
  const thread = threads.find((entry) => entry.key === "thread-state-1");
  assert.ok(thread);
  assert.equal(thread.lastFamily, "summarization");
  assert.equal(thread.turnCount, 1);
});
