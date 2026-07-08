import test from "node:test";
import assert from "node:assert/strict";

import { handleLlmCompletions } from "../llm.mjs";
import { handleIntegrations } from "../integrations.mjs";
import {
  applyDynamicPlaceholdersToResponse,
  renderDynamicPlaceholders,
} from "../llm/shared.mjs";
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

// ── {{DYNAMIC_*}} placeholder substitution (#4517) ──────────────────
//
// Envelope shape matches awaiting_user_envelope() in
// src/openhuman/agent_orchestration/tools/awaiting_user.rs — the parser
// must handle the exact production format (JSON-encoded question,
// `worker_thread_id: (none)`, trailing instruction block).
function subagentAwaitingUserEnvelope({
  taskId = "sub-abc123-fake-uuid",
  agentId = "researcher",
  workerThreadId = "(none)",
  question = "Which repo should I search?",
} = {}) {
  return `[SUBAGENT_AWAITING_USER]
task_id: ${taskId}
agent_id: ${agentId}
worker_thread_id: ${workerThreadId}
question: ${JSON.stringify(question)}
[/SUBAGENT_AWAITING_USER]

The sub-agent needs clarification before it can continue. Surface the above question to the user. When the user responds, call continue_subagent with the task_id, agent_id, and the user's answer as the message parameter.`;
}

test("renderDynamicPlaceholders substitutes task_id and agent_id from history", () => {
  const parsedBody = {
    messages: [
      { role: "user", content: "please research the codex marker" },
      {
        role: "tool",
        content: subagentAwaitingUserEnvelope({
          taskId: "sub-runtime-42",
          agentId: "researcher",
        }),
      },
      { role: "user", content: "the main repo" },
    ],
  };
  const rendered = renderDynamicPlaceholders(
    '{"task_id":"{{DYNAMIC_TASK_ID}}","agent_id":"{{DYNAMIC_AGENT_ID}}","message":"the main repo"}',
    parsedBody,
  );
  assert.equal(
    rendered,
    '{"task_id":"sub-runtime-42","agent_id":"researcher","message":"the main repo"}',
  );
});

test("renderDynamicPlaceholders returns input unchanged when no envelope present", () => {
  const parsedBody = {
    messages: [{ role: "user", content: "no envelope here" }],
  };
  assert.equal(
    renderDynamicPlaceholders("{{DYNAMIC_TASK_ID}}", parsedBody),
    "{{DYNAMIC_TASK_ID}}",
  );
});

test("renderDynamicPlaceholders short-circuits when text has no placeholders", () => {
  // Guardrail: no envelope scan for content without placeholders.
  assert.equal(renderDynamicPlaceholders("plain text", null), "plain text");
});

test("applyDynamicPlaceholdersToResponse renders content and toolCalls arguments without mutating input", () => {
  const parsedBody = {
    messages: [
      {
        role: "tool",
        content: subagentAwaitingUserEnvelope({ taskId: "sub-xyz" }),
      },
    ],
  };
  const original = {
    content: "prefix {{DYNAMIC_TASK_ID}}",
    toolCalls: [
      {
        id: "call_1",
        name: "continue_subagent",
        arguments: '{"task_id":"{{DYNAMIC_TASK_ID}}"}',
      },
    ],
  };
  const rendered = applyDynamicPlaceholdersToResponse(original, parsedBody);
  assert.equal(rendered.content, "prefix sub-xyz");
  assert.equal(rendered.toolCalls[0].arguments, '{"task_id":"sub-xyz"}');
  // Purity: input must not be mutated (call sites re-read the rule on the
  // next request; mutation would let a first substitution leak into later
  // turns that don't have an envelope).
  assert.equal(original.content, "prefix {{DYNAMIC_TASK_ID}}");
  assert.equal(
    original.toolCalls[0].arguments,
    '{"task_id":"{{DYNAMIC_TASK_ID}}"}',
  );
});

test("llmKeywordRules substitute {{DYNAMIC_TASK_ID}} in continue_subagent tool_call args", () => {
  setMockBehaviors(
    {
      llmKeywordRules: JSON.stringify([
        {
          keyword: "user answer",
          content: "",
          toolCalls: [
            {
              id: "call_continue_1",
              name: "continue_subagent",
              arguments: JSON.stringify({
                task_id: "{{DYNAMIC_TASK_ID}}",
                agent_id: "{{DYNAMIC_AGENT_ID}}",
                message: "the main repo",
              }),
            },
          ],
        },
      ]),
    },
    "replace",
  );

  const ctx = makeCtx({
    parsedBody: {
      model: "e2e-mock-model",
      // No tools → not a "primary turn" for the forced-response FIFO, but
      // keyword rules still fire and this validates the substitution site.
      messages: [
        {
          role: "tool",
          content: subagentAwaitingUserEnvelope({
            taskId: "sub-realtime-777",
            agentId: "researcher",
          }),
        },
        { role: "user", content: "here is my user answer" },
      ],
    },
  });

  assert.equal(handleLlmCompletions(ctx), true);
  const body = JSON.parse(ctx.res.body);
  const args = body.choices[0].message.tool_calls[0].function.arguments;
  const parsed = JSON.parse(args);
  assert.equal(parsed.task_id, "sub-realtime-777");
  assert.equal(parsed.agent_id, "researcher");
  assert.equal(parsed.message, "the main repo");
  // Placeholder text must be fully consumed.
  assert.ok(!args.includes("{{DYNAMIC_"), `args should not contain unresolved placeholders: ${args}`);
});

test("llmKeywordRules leave the rule unmutated across successive requests", () => {
  // Regression guard for applyDynamicPlaceholdersToResponse purity: two
  // requests, only the first has an envelope in history. Second must still
  // see the raw `{{DYNAMIC_TASK_ID}}` (which then falls through unchanged
  // since there is no envelope to substitute from).
  setMockBehaviors(
    {
      llmKeywordRules: JSON.stringify([
        {
          keyword: "answer",
          content: "raw {{DYNAMIC_TASK_ID}}",
        },
      ]),
    },
    "replace",
  );

  const first = makeCtx({
    parsedBody: {
      model: "e2e-mock-model",
      messages: [
        {
          role: "tool",
          content: subagentAwaitingUserEnvelope({ taskId: "sub-first" }),
        },
        { role: "user", content: "my answer" },
      ],
    },
  });
  assert.equal(handleLlmCompletions(first), true);
  const firstBody = JSON.parse(first.res.body);
  assert.equal(firstBody.choices[0].message.content, "raw sub-first");

  const second = makeCtx({
    parsedBody: {
      model: "e2e-mock-model",
      messages: [{ role: "user", content: "another answer" }],
    },
  });
  assert.equal(handleLlmCompletions(second), true);
  const secondBody = JSON.parse(second.res.body);
  // No envelope → helper leaves `{{DYNAMIC_TASK_ID}}` verbatim (identity
  // fast-path). Prior substitution must not have poisoned the stored rule.
  assert.equal(secondBody.choices[0].message.content, "raw {{DYNAMIC_TASK_ID}}");
});
