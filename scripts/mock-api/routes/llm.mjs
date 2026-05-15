import { json } from "../http.mjs";
import { behavior, parseBehaviorJson, setMockBehavior } from "../state.mjs";

/**
 * Smart mock LLM endpoint.
 *
 * Drives keyword-based routing so unit/E2E tests can exercise the agent
 * harness end-to-end without spinning up a real model. The mock looks
 * at the latest user/tool message in the request and either:
 *
 *  1. Replays a forced response queue (`llmForcedResponses` behavior),
 *  2. Matches a configured keyword rule (`llmKeywordRules` behavior),
 *  3. Falls through to a sensible default ("Hello from e2e mock agent").
 *
 * Keyword rules look like:
 *
 *   [
 *     {
 *       "keyword": "search",                       // case-insensitive substring
 *       "toolCalls": [
 *         { "name": "search_tool", "arguments": {"q": "rust"} }
 *       ],
 *       "content": "Looking it up..."
 *     },
 *     {
 *       "keyword": "search_tool-ok",
 *       "content": "Here's the answer."
 *     }
 *   ]
 *
 * Configure with:
 *   POST /__admin/behavior  body: {"llmKeywordRules": "<json-string>"}
 *
 * This mirrors the Rust-side `KeywordScriptedProvider` in
 * `src/openhuman/agent/harness/test_support.rs` so the same testing
 * mental model applies on both sides of the FFI.
 */

function pickProbeText(parsedBody) {
  if (!parsedBody || !Array.isArray(parsedBody.messages)) return "";
  for (let i = parsedBody.messages.length - 1; i >= 0; i -= 1) {
    const m = parsedBody.messages[i];
    if (!m || typeof m !== "object") continue;
    if (m.role === "user" || m.role === "tool") {
      if (typeof m.content === "string") return m.content;
      if (Array.isArray(m.content)) {
        return m.content
          .filter((c) => c && c.type === "text" && typeof c.text === "string")
          .map((c) => c.text)
          .join(" ");
      }
    }
  }
  return "";
}

function makeChoice({ content, toolCalls, callIdSeed }) {
  const message = { role: "assistant", content: content ?? "" };
  if (Array.isArray(toolCalls) && toolCalls.length > 0) {
    message.tool_calls = toolCalls.map((tc, idx) => ({
      id: tc.id ?? `call_${callIdSeed}_${idx}`,
      type: "function",
      function: {
        name: String(tc.name ?? ""),
        arguments:
          typeof tc.arguments === "string"
            ? tc.arguments
            : JSON.stringify(tc.arguments ?? {}),
      },
    }));
    if (!content) message.content = null;
  }
  return { index: 0, message, finish_reason: toolCalls?.length ? "tool_calls" : "stop" };
}

function buildResponse({ model, content, toolCalls }) {
  const seed = Date.now();
  return {
    id: `chatcmpl-mock-${seed}`,
    object: "chat.completion",
    created: Math.floor(seed / 1000),
    model: model || "e2e-mock-model",
    choices: [makeChoice({ content, toolCalls, callIdSeed: seed })],
    usage: {
      prompt_tokens: 10,
      completion_tokens: 10,
      total_tokens: 20,
    },
  };
}

/**
 * Drive a mock OpenAI-compatible /v1/chat/completions endpoint with
 * keyword-based responses. Returns true if the request was handled.
 */
export function handleLlmCompletions(ctx) {
  const { method, url, parsedBody, res } = ctx;
  if (
    method !== "POST" ||
    !/^\/openai\/v1\/chat\/completions\/?$/.test(url)
  ) {
    return false;
  }

  const mockBehavior = behavior();
  const model =
    typeof parsedBody?.model === "string" ? parsedBody.model : "e2e-mock-model";

  // 1. Forced queue — replay exact ChatResponse objects in order.
  const forced = parseBehaviorJson("llmForcedResponses", []);
  if (Array.isArray(forced) && forced.length > 0) {
    const next = forced.shift();
    // Persist the shrunk queue back so subsequent requests advance.
    setMockBehavior("llmForcedResponses", JSON.stringify(forced));
    json(res, 200, buildResponse({ model, ...next }));
    return true;
  }

  // 2. Keyword rules.
  const rules = parseBehaviorJson("llmKeywordRules", []);
  const probe = pickProbeText(parsedBody).toLowerCase();
  if (Array.isArray(rules)) {
    for (const rule of rules) {
      if (!rule || typeof rule.keyword !== "string") continue;
      if (probe.includes(rule.keyword.toLowerCase())) {
        json(
          res,
          200,
          buildResponse({
            model,
            content: rule.content ?? "",
            toolCalls: rule.toolCalls ?? [],
          }),
        );
        return true;
      }
    }
  }

  // 3. Default fallback.
  const fallback =
    typeof mockBehavior.llmFallbackContent === "string" &&
    mockBehavior.llmFallbackContent.length > 0
      ? mockBehavior.llmFallbackContent
      : "Hello from e2e mock agent";
  json(res, 200, buildResponse({ model, content: fallback }));
  return true;
}
