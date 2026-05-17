import { json, setCors } from "../http.mjs";
import {
  behavior,
  getMockLlmThread,
  parseBehaviorJson,
  recordMockLlmTurn,
  setMockBehavior,
} from "../state.mjs";
import { buildDynamicCompletion } from "./llm/dynamic.mjs";
import { headerValue, pickProbeText, resolveThreadKey } from "./llm/shared.mjs";

function requestRuleMatches(rule, ctx) {
  if (!rule || typeof rule !== "object") return false;
  const { url, parsedBody, req } = ctx;
  const model =
    typeof parsedBody?.model === "string" ? parsedBody.model : "e2e-mock-model";
  const stream = parsedBody?.stream === true;
  const authorization = headerValue(req?.headers, "authorization");
  const xApiKey = headerValue(req?.headers, "x-api-key");

  if (typeof rule.path === "string" && rule.path !== url) return false;
  if (typeof rule.model === "string" && rule.model !== model) return false;
  if (typeof rule.stream === "boolean" && rule.stream !== stream) return false;

  if (typeof rule.authorization === "string") {
    if (rule.authorization === "present" && !authorization) return false;
    if (rule.authorization === "missing" && authorization) return false;
    if (
      rule.authorization !== "present" &&
      rule.authorization !== "missing" &&
      authorization !== rule.authorization
    ) {
      return false;
    }
  }

  if (typeof rule.xApiKey === "string") {
    if (rule.xApiKey === "present" && !xApiKey) return false;
    if (rule.xApiKey === "missing" && xApiKey) return false;
    if (
      rule.xApiKey !== "present" &&
      rule.xApiKey !== "missing" &&
      xApiKey !== rule.xApiKey
    ) {
      return false;
    }
  }

  if (typeof rule.keyword === "string") {
    const probe = pickProbeText(parsedBody).toLowerCase();
    if (!probe.includes(rule.keyword.toLowerCase())) return false;
  }

  return true;
}

function resolveRequestRule(ctx) {
  const rules = parseBehaviorJson("llmRequestRules", []);
  if (!Array.isArray(rules)) return null;
  for (const rule of rules) {
    if (requestRuleMatches(rule, ctx)) return rule;
  }
  return null;
}

function sendRuleError(res, rule) {
  const status = Number.isInteger(rule?.status) ? rule.status : 401;
  const message =
    typeof rule?.error === "string" && rule.error.length > 0
      ? rule.error
      : "mock LLM request rejected";
  json(res, status, {
    error: {
      message,
      type: rule?.type || "invalid_request_error",
      code: rule?.code || null,
    },
  });
}

// ── Streaming helpers ─────────────────────────────────────────────
//
// When the agent harness calls the OpenAI-compatible endpoint with
// `stream: true`, openhuman/providers/compatible.rs expects SSE chunks
// shaped like:
//
//   data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}
//   data: {"choices":[{"delta":{},"finish_reason":"stop"}], "usage":{...}}
//   data: [DONE]
//
// The streaming branch is configured via two mock behavior keys:
//
//   llmStreamScript        — JSON array of script entries (see below).
//                            Overrides everything else when present.
//   llmStreamChunkDelayMs  — default delay between chunks (ms).
//
// Script entry shapes:
//   { "text": "Hello",      "delayMs": 30 }                       text delta
//   { "thinking": "...",    "delayMs": 30 }                       reasoning delta
//   { "toolCall": { "id": "call_x", "name": "foo", "arguments": "{\"a\":1}" } }
//                                                                 emits a tool_call
//                                                                 start chunk plus
//                                                                 incremental args
//                                                                 chunks (split by
//                                                                 8-char windows)
//   { "finish": "stop" | "tool_calls" }                           final empty chunk
//   { "usage": {"prompt_tokens":1,"completion_tokens":2,"total_tokens":3} }
//                                                                 attached to the
//                                                                 last emitted chunk
//   { "error": "kaboom" }                                         emits an error
//                                                                 SSE event and
//                                                                 closes the
//                                                                 connection (no
//                                                                 [DONE])
//
// If no `llmStreamScript` is set, a keyword rule matching the latest
// user message is auto-converted into a script. If no rule matches we
// stream a default greeting in three deltas so basic streaming UI
// behavior is exercised even with zero configuration.

function writeSseHead(res) {
  setCors(res);
  res.writeHead(200, {
    "Content-Type": "text/event-stream; charset=utf-8",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
}

function sseChunkEnvelope({
  model,
  contentDelta,
  thinkingDelta,
  toolCallDelta,
  finishReason,
  usage,
}) {
  const choice = {
    index: 0,
    delta: {},
    finish_reason: finishReason ?? null,
  };
  if (typeof contentDelta === "string") choice.delta.content = contentDelta;
  if (typeof thinkingDelta === "string")
    choice.delta.reasoning_content = thinkingDelta;
  if (toolCallDelta) choice.delta.tool_calls = [toolCallDelta];

  const envelope = {
    id: `chatcmpl-mock-${Date.now()}`,
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model: model || "e2e-mock-model",
    choices: [choice],
  };
  if (usage) envelope.usage = usage;
  return envelope;
}

function writeSseEvent(res, payload) {
  res.write(`data: ${JSON.stringify(payload)}\n\n`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// Split a string into N-character windows so we can stream tool-call
// argument JSON the same way real providers do — clients accumulate the
// partial fragments and JSON-parse at the end.
function chunkString(s, windowSize) {
  const out = [];
  if (!s) return out;
  for (let i = 0; i < s.length; i += windowSize) {
    out.push(s.slice(i, i + windowSize));
  }
  return out;
}

function defaultStreamScript({ content, toolCalls }) {
  const script = [];
  // Real OpenAI streams a text preamble (when present) BEFORE tool-call
  // deltas; collapsing that to nothing the moment tool_calls show up
  // would diverge from the non-streaming `{ content, toolCalls }`
  // contract and silently drop assistant-visible reasoning.
  const text =
    typeof content === "string" && content.length > 0 ? content : null;

  if (Array.isArray(toolCalls) && toolCalls.length > 0) {
    if (text) {
      for (const piece of chunkString(text, 12)) {
        script.push({ text: piece });
      }
    }
    for (let i = 0; i < toolCalls.length; i += 1) {
      const tc = toolCalls[i];
      script.push({
        toolCall: {
          // `index` is what the OpenAI streaming protocol uses to
          // demux multiple parallel tool calls. Preserve it here so a
          // single-script entry with N tool calls becomes N distinct
          // calls on the client side instead of being reassembled
          // into one.
          index: i,
          id: tc.id ?? `call_stream_${i}`,
          name: String(tc.name ?? ""),
          arguments:
            typeof tc.arguments === "string"
              ? tc.arguments
              : JSON.stringify(tc.arguments ?? {}),
        },
      });
    }
    script.push({ finish: "tool_calls" });
    return script;
  }
  const fallbackText = text ?? "Hello from e2e mock agent";
  // Split into ~12-char windows so a UI-side delta watcher sees several
  // arrival events even for short responses.
  for (const piece of chunkString(fallbackText, 12)) {
    script.push({ text: piece });
  }
  script.push({ finish: "stop" });
  return script;
}

function handleStreamingCompletion({
  res,
  model,
  mockBehavior,
  parsedBody,
  rule,
  dynamic,
}) {
  writeSseHead(res);

  // 1. Explicit streaming script overrides everything.
  let script = Array.isArray(rule?.streamScript) ? rule.streamScript : null;

  if (!Array.isArray(script)) {
    script = parseBehaviorJson("llmStreamScript", null);
  }

  if (!Array.isArray(script)) {
    // 2. Forced queue: pop the next entry and convert it into a script.
    const forced = parseBehaviorJson("llmForcedResponses", []);
    if (Array.isArray(forced) && forced.length > 0) {
      const next = forced.shift();
      setMockBehavior("llmForcedResponses", JSON.stringify(forced));
      script = defaultStreamScript({
        content: next.content,
        toolCalls: next.toolCalls,
      });
    }
  }

  if (!Array.isArray(script)) {
    // 3. Keyword rules — match on latest user/tool message.
    const rules = parseBehaviorJson("llmKeywordRules", []);
    const probe = pickProbeText(parsedBody).toLowerCase();
    if (Array.isArray(rules)) {
      for (const rule of rules) {
        if (!rule || typeof rule.keyword !== "string") continue;
        if (probe.includes(rule.keyword.toLowerCase())) {
          script = defaultStreamScript({
            content: rule.content,
            toolCalls: rule.toolCalls,
          });
          break;
        }
      }
    }
  }

  if (!Array.isArray(script)) {
    // 4. Default: stream a short greeting in a few chunks.
    if (
      Array.isArray(dynamic?.streamScript) &&
      dynamic.streamScript.length > 0
    ) {
      script = dynamic.streamScript;
    } else {
      const fallback =
        typeof rule?.content === "string" && rule.content.length > 0
          ? rule.content
          : typeof mockBehavior.llmFallbackContent === "string" &&
              mockBehavior.llmFallbackContent.length > 0
            ? mockBehavior.llmFallbackContent
            : "Hello from e2e mock agent";
      script = defaultStreamScript({
        content: fallback,
        toolCalls: Array.isArray(rule?.toolCalls) ? rule.toolCalls : undefined,
      });
    }
  }

  const defaultDelayMs = Number.isFinite(
    parseFloat(mockBehavior.llmStreamChunkDelayMs),
  )
    ? Math.max(0, parseFloat(mockBehavior.llmStreamChunkDelayMs))
    : 25;

  // Fire-and-forget — the dispatcher only cares that the handler
  // claimed the request. Errors mid-stream are surfaced through SSE.
  streamScriptToResponse({ res, model, script, defaultDelayMs }).catch(
    (err) => {
      try {
        writeSseEvent(res, {
          error: { message: `mock stream error: ${err?.message ?? err}` },
        });
      } catch {
        // ignore — connection likely already closed
      }
      try {
        res.end();
      } catch {
        // ignore
      }
    },
  );

  return true;
}

async function streamScriptToResponse({ res, model, script, defaultDelayMs }) {
  let trailingUsage = null;
  for (let i = 0; i < script.length; i += 1) {
    const entry = script[i] ?? {};
    const delay = Number.isFinite(entry.delayMs)
      ? entry.delayMs
      : defaultDelayMs;
    if (delay > 0) await sleep(delay);

    if (entry.error) {
      writeSseEvent(res, { error: { message: String(entry.error) } });
      res.end();
      return;
    }

    if (entry.usage && typeof entry.usage === "object") {
      // Buffer usage until the next chunk that carries finish_reason.
      trailingUsage = entry.usage;
      continue;
    }

    if (typeof entry.text === "string") {
      writeSseEvent(res, sseChunkEnvelope({ model, contentDelta: entry.text }));
      continue;
    }

    if (typeof entry.thinking === "string") {
      writeSseEvent(
        res,
        sseChunkEnvelope({ model, thinkingDelta: entry.thinking }),
      );
      continue;
    }

    if (entry.toolCall) {
      const tc = entry.toolCall;
      // Preserve the caller-supplied index when present. Real OpenAI
      // streams use `index` to demux multiple parallel tool calls in
      // the same message — collapsing every delta to `index: 0`
      // breaks the multi-tool reassembly contract on the client.
      const index = Number.isInteger(tc.index) ? tc.index : 0;
      const id = tc.id ?? `call_stream_${i}`;
      const name = String(tc.name ?? "");
      const argsRaw =
        typeof tc.arguments === "string"
          ? tc.arguments
          : JSON.stringify(tc.arguments ?? {});

      // Opening chunk: carries id + name + first arg fragment.
      const argPieces = chunkString(argsRaw, 8);
      const first = argPieces.shift() ?? "";
      writeSseEvent(
        res,
        sseChunkEnvelope({
          model,
          toolCallDelta: {
            index,
            id,
            type: "function",
            function: { name, arguments: first },
          },
        }),
      );
      for (const piece of argPieces) {
        if (delay > 0) await sleep(delay);
        writeSseEvent(
          res,
          sseChunkEnvelope({
            model,
            toolCallDelta: {
              index,
              function: { arguments: piece },
            },
          }),
        );
      }
      continue;
    }

    if (entry.finish) {
      writeSseEvent(
        res,
        sseChunkEnvelope({
          model,
          finishReason: entry.finish,
          usage: trailingUsage ?? undefined,
        }),
      );
      trailingUsage = null;
      continue;
    }
  }

  // Always close with the [DONE] sentinel — clients use it to detect
  // graceful end-of-stream and clear in-flight state. Skipping it
  // wedges the in-flight map until the next request lands.
  res.write("data: [DONE]\n\n");
  res.end();
}

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
  return {
    index: 0,
    message,
    finish_reason: toolCalls?.length ? "tool_calls" : "stop",
  };
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
 * Drive a mock OpenAI-compatible chat-completions endpoint with
 * keyword-based responses. Accepts both `/v1/chat/completions` and
 * root-based `/chat/completions` URLs because custom providers often
 * let users enter either the API root or an explicit `/v1` base.
 * Returns true if the request was handled.
 */
export function handleLlmCompletions(ctx) {
  const { method, url, parsedBody, res } = ctx;
  if (
    method !== "POST" ||
    !/^(\/openai)?(\/v1)?\/chat\/completions\/?$/.test(url)
  ) {
    return false;
  }

  const mockBehavior = behavior();
  const model =
    typeof parsedBody?.model === "string" ? parsedBody.model : "e2e-mock-model";
  const requestRule = resolveRequestRule(ctx);
  const threadKey = resolveThreadKey(ctx);
  const thread = threadKey ? getMockLlmThread(threadKey) : null;
  const dynamic = buildDynamicCompletion({ model, parsedBody, thread });
  const requestText = pickProbeText(parsedBody);

  if (
    requestRule?.error ||
    (requestRule?.status && requestRule.status >= 400)
  ) {
    if (
      parsedBody?.stream === true &&
      requestRule?.error &&
      !(Number.isInteger(requestRule?.status) && requestRule.status >= 400)
    ) {
      writeSseHead(res);
      writeSseEvent(res, {
        error: {
          message: requestRule?.error || "mock LLM streaming request rejected",
          type: requestRule?.type || "invalid_request_error",
          code: requestRule?.code || null,
        },
      });
      res.end();
      return true;
    }
    sendRuleError(res, requestRule);
    return true;
  }

  // ── Streaming branch ────────────────────────────────────────────
  // Drive the OpenAI SSE protocol when the caller requested it. The
  // agent harness sets `stream: true` whenever it has a delta channel
  // attached, which is the production code path — non-streaming is
  // only the OH-backend fallback. See compatible.rs `chat()`.
  if (parsedBody?.stream === true) {
    const recordTurn = (result) => {
      if (!threadKey) return;
      recordMockLlmTurn(threadKey, {
        requestText,
        responseText: result?.content ?? null,
        toolCalls: result?.toolCalls ?? [],
        model,
        family: result?.family ?? dynamic.family,
        codeLanguage: result?.codeLanguage ?? null,
      });
    };

    if (
      requestRule?.streamScript ||
      requestRule?.content ||
      requestRule?.toolCalls
    ) {
      recordTurn({
        content: requestRule?.content ?? "",
        toolCalls: requestRule?.toolCalls ?? [],
        family: dynamic.family,
      });
    } else {
      recordTurn(dynamic);
    }
    return handleStreamingCompletion({
      res,
      model,
      mockBehavior,
      parsedBody,
      rule: requestRule,
      dynamic,
    });
  }

  if (requestRule?.body && typeof requestRule.body === "object") {
    json(
      res,
      Number.isInteger(requestRule.status) ? requestRule.status : 200,
      requestRule.body,
    );
    return true;
  }

  if (
    Array.isArray(requestRule?.toolCalls) ||
    typeof requestRule?.content === "string"
  ) {
    if (threadKey) {
      recordMockLlmTurn(threadKey, {
        requestText,
        responseText: requestRule?.content ?? "",
        toolCalls: requestRule?.toolCalls ?? [],
        model,
        family: dynamic.family,
      });
    }
    json(
      res,
      Number.isInteger(requestRule?.status) ? requestRule.status : 200,
      buildResponse({
        model,
        content: requestRule?.content ?? "",
        toolCalls: requestRule?.toolCalls ?? [],
      }),
    );
    return true;
  }

  // 1. Forced queue — replay exact ChatResponse objects in order.
  const forced = parseBehaviorJson("llmForcedResponses", []);
  if (Array.isArray(forced) && forced.length > 0) {
    const next = forced.shift();
    // Persist the shrunk queue back so subsequent requests advance.
    setMockBehavior("llmForcedResponses", JSON.stringify(forced));
    if (threadKey) {
      recordMockLlmTurn(threadKey, {
        requestText,
        responseText: next.content ?? "",
        toolCalls: next.toolCalls ?? [],
        model,
        family: dynamic.family,
      });
    }
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
        if (threadKey) {
          recordMockLlmTurn(threadKey, {
            requestText,
            responseText: rule.content ?? "",
            toolCalls: rule.toolCalls ?? [],
            model,
            family: dynamic.family,
          });
        }
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
    dynamic.content ||
    (typeof mockBehavior.llmFallbackContent === "string" &&
    mockBehavior.llmFallbackContent.length > 0
      ? mockBehavior.llmFallbackContent
      : "Hello from e2e mock agent");
  if (threadKey) {
    recordMockLlmTurn(threadKey, {
      requestText,
      responseText: fallback,
      toolCalls: dynamic.toolCalls ?? [],
      toolResultText: null,
      model,
      family: dynamic.family,
      codeLanguage: dynamic.codeLanguage ?? null,
    });
  }
  json(
    res,
    200,
    buildResponse({
      model,
      content: fallback,
      toolCalls: dynamic.toolCalls ?? [],
    }),
  );
  return true;
}
