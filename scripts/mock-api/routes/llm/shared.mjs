import { behavior, parseBehaviorJson } from "../../state.mjs";

const DEFAULT_MODELS = [
  { id: "e2e-mock-model", family: "chat", owned_by: "openhuman-mock" },
  {
    id: "openhuman-reasoning-mock",
    family: "reasoning",
    owned_by: "openhuman-mock",
  },
  { id: "o3-mini-mock", family: "reasoning", owned_by: "openhuman-mock" },
  {
    id: "openhuman-agentic-mock",
    family: "agentic",
    owned_by: "openhuman-mock",
  },
  { id: "gpt-4.1-agent-mock", family: "agentic", owned_by: "openhuman-mock" },
  { id: "openhuman-coder-mock", family: "coding", owned_by: "openhuman-mock" },
  { id: "gpt-5-codex-mock", family: "coding", owned_by: "openhuman-mock" },
  {
    id: "openhuman-summary-mock",
    family: "summarization",
    owned_by: "openhuman-mock",
  },
  {
    id: "memory-summarizer-mock",
    family: "summarization",
    owned_by: "openhuman-mock",
  },
];

export function listMockLlmModels() {
  const configured = parseBehaviorJson("llmModelCatalog", null);
  const catalog =
    Array.isArray(configured) && configured.length > 0
      ? configured
      : DEFAULT_MODELS;
  return catalog.map((entry, index) => ({
    id: String(entry.id || `mock-model-${index + 1}`),
    object: "model",
    created: entry.created || 1_710_000_000,
    owned_by: entry.owned_by || entry.ownedBy || "openhuman-mock",
    family:
      entry.family ||
      detectModelFamily({
        model: String(entry.id || `mock-model-${index + 1}`),
      }),
  }));
}

export function headerValue(headers, name) {
  const raw = headers?.[name];
  if (Array.isArray(raw)) return raw.join(", ");
  return typeof raw === "string" ? raw : "";
}

export function pickProbeText(parsedBody) {
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

export function normalizeMessageContent(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter(
      (item) => item && item.type === "text" && typeof item.text === "string",
    )
    .map((item) => item.text)
    .join(" ");
}

export function collectMessagesByRole(parsedBody, role) {
  if (!Array.isArray(parsedBody?.messages)) return [];
  return parsedBody.messages
    .filter((message) => message?.role === role)
    .map((message) => ({
      ...message,
      normalizedContent: normalizeMessageContent(message.content),
    }));
}

export function latestRoleMessage(parsedBody, role) {
  const matches = collectMessagesByRole(parsedBody, role);
  return matches[matches.length - 1] || null;
}

export function resolveThreadKey(ctx) {
  const { parsedBody, req } = ctx;
  const headers = req?.headers || {};
  const body = parsedBody || {};
  return (
    headerValue(headers, "x-mock-thread-id") ||
    headerValue(headers, "x-thread-id") ||
    body.mockThreadId ||
    body.threadId ||
    body.conversationId ||
    body.sessionId ||
    body.metadata?.thread_id ||
    body.metadata?.conversation_id ||
    body.metadata?.session_id ||
    body.user ||
    null
  );
}

function overrideFamilyForModel(model) {
  const overrides = parseBehaviorJson("llmModelFamilyOverrides", []);
  if (!Array.isArray(overrides)) return null;
  for (const entry of overrides) {
    if (!entry || typeof entry.family !== "string") continue;
    if (typeof entry.model === "string" && entry.model === model) {
      return entry.family;
    }
    if (
      typeof entry.match === "string" &&
      model.includes(entry.match.toLowerCase())
    ) {
      return entry.family;
    }
  }
  return null;
}

export function detectModelFamily({ model = "", parsedBody } = {}) {
  const lower = String(model || "").toLowerCase();
  const override = overrideFamilyForModel(lower);
  if (override) return override;
  if (/codex|coder|code|devstral|program|repair/.test(lower)) return "coding";
  if (/summary|summar|memory|brief|distill|extract/.test(lower)) {
    return "summarization";
  }
  if (/agent|tool|operator|workflow|computer|action/.test(lower)) {
    return "agentic";
  }
  if (/reason|thinking|o1|o3|o4|r1|deepseek/.test(lower)) return "reasoning";
  if (Array.isArray(parsedBody?.tools) && parsedBody.tools.length > 0) {
    return "agentic";
  }
  return behavior().llmDefaultFamily || "chat";
}
