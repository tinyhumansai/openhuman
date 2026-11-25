import { json } from "../http.mjs";
import { behavior, parseBehaviorJson, setMockBehavior } from "../state.mjs";
import { listMockLlmModels } from "./llm/shared.mjs";

// The web E2E core must never fetch the public Hermes catalog. Keep the
// fixture representative: registry smoke tests need sources and browse
// results, search tests need both `git` and `docker` matches, and the
// explorer needs more than one 25-row page to exercise server paging.
const SKILL_REGISTRY_FILLER = Array.from({ length: 30 }, (_, i) => ({
  name: `fixture-skill-${String(i).padStart(2, "0")}`,
  description: "Registry paging fixture.",
  category: "productivity",
  source: "fixture-pack",
  tags: [],
  platforms: ["linux", "macos", "windows"],
  commands: [],
  envVars: [],
}));

const SKILL_REGISTRY_CATALOG = [
  {
    name: "git-workflow",
    description: "Inspect repositories and manage Git branches.",
    category: "development",
    source: "built-in",
    docsPath: "bundled/development/git-workflow",
    tags: ["git", "repository"],
    platforms: ["linux", "macos", "windows"],
    commands: ["git"],
    envVars: [],
  },
  {
    name: "docker-management",
    description: "Inspect and manage Docker containers.",
    category: "devops",
    source: "optional",
    docsPath: "optional/devops/devops-docker-management",
    tags: ["docker", "containers"],
    platforms: ["linux", "macos", "windows"],
    commands: ["docker"],
    envVars: [],
  },
  ...SKILL_REGISTRY_FILLER,
];

// The skill named by `skillRegistryScanBlocked` carries a zero-width space,
// which the supply-chain scan blocks; `skillRegistryScanVariant` changes its
// text, and so its digest.
function skillRegistryDocument(name, scanBlocked, variant) {
  const suffix = variant ? ` (${variant})` : "";
  const body = scanBlocked ? `Run the steps\u200b in order.${suffix}\n` : "";
  return `---\nname: ${name}\ndescription: Mock registry skill ${name}.\n---\n\n# ${name}\n${body}`;
}

export function handleIntegrations(ctx) {
  const { method, url, parsedBody, res } = ctx;
  const mockBehavior = behavior();

  if (method === "GET" && /^\/skills\/catalog\.json\/?(?:\?.*)?$/.test(url)) {
    if (mockBehavior.skillRegistryUnavailable === "true") {
      json(res, 503, { success: false, error: "skill registry unavailable" });
      return true;
    }
    json(res, 200, SKILL_REGISTRY_CATALOG);
    return true;
  }

  const skillDocument =
    method === "GET" &&
    url.match(/^\/skills\/([a-z0-9-]+)\/SKILL\.md(?:\?.*)?$/);
  if (skillDocument) {
    const name = skillDocument[1];
    if (!SKILL_REGISTRY_CATALOG.some((entry) => entry.name === name)) {
      json(res, 404, { success: false, error: "no such skill" });
      return true;
    }
    res.writeHead(200, { "Content-Type": "text/markdown; charset=utf-8" });
    res.end(
      skillRegistryDocument(
        name,
        mockBehavior.skillRegistryScanBlocked === name,
        mockBehavior.skillRegistryScanVariant ?? "",
      ),
    );
    return true;
  }

  // ── Telegram ───────────────────────────────────────────────
  if (method === "POST" && /^\/telegram\/command\/?$/.test(url)) {
    if (mockBehavior.telegramUnauthorized === "true") {
      json(res, 403, {
        success: false,
        error: "Unauthorized: insufficient permissions",
      });
      return true;
    }
    if (mockBehavior.telegramCommandError === "true") {
      json(res, 400, { success: false, error: "Invalid command format" });
      return true;
    }
    json(res, 200, {
      success: true,
      data: { result: "Command executed successfully" },
    });
    return true;
  }

  if (method === "GET" && /^\/telegram\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior.telegramPermission || "read";
    json(res, 200, {
      success: true,
      data: {
        level,
        canRead: true,
        canWrite: level !== "read",
        canInitiate: level === "admin",
      },
    });
    return true;
  }

  if (method === "POST" && /^\/telegram\/webhook\/configure\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        webhookUrl: "https://api.example.com/webhook/telegram",
        active: true,
      },
    });
    return true;
  }

  if (method === "POST" && /^\/telegram\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return true;
  }

  // ── Notion ─────────────────────────────────────────────────
  if (method === "GET" && /^\/notion\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior.notionPermission || "read";
    json(res, 200, {
      success: true,
      data: {
        level,
        canRead: true,
        canWrite: level !== "read",
        canCreate: level !== "read",
      },
    });
    return true;
  }

  // ── Gmail ──────────────────────────────────────────────────
  if (method === "GET" && /^\/gmail\/permissions\/?(\?.*)?$/.test(url)) {
    const level = mockBehavior.gmailPermission || "read";
    json(res, 200, {
      success: true,
      data: {
        level,
        canRead: true,
        canWrite: level !== "read",
        canInitiate: level === "admin",
      },
    });
    return true;
  }

  if (method === "POST" && /^\/gmail\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return true;
  }

  if (method === "GET" && /^\/gmail\/emails\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: [
        {
          id: "msg-1",
          subject: "Welcome to OpenHuman",
          from: "team@openhuman.com",
          date: new Date().toISOString(),
          snippet: "Welcome to the platform!",
          hasAttachments: false,
        },
      ],
    });
    return true;
  }

  // ── Skills ─────────────────────────────────────────────────
  if (method === "GET" && /^\/skills\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: [
        {
          id: "telegram",
          name: "Telegram",
          status: mockBehavior.telegramSkillStatus || "installed",
          setupComplete: mockBehavior.telegramSetupComplete === "true",
        },
        {
          id: "notion",
          name: "Notion",
          status: mockBehavior.notionSkillStatus || "installed",
          setupComplete: mockBehavior.notionSetupComplete === "true",
        },
        {
          id: "email",
          name: "Email",
          status: mockBehavior.gmailSkillStatus || "installed",
          setupComplete: mockBehavior.gmailSetupComplete === "true",
        },
      ],
    });
    return true;
  }

  // ── OpenAI proxy ───────────────────────────────────────────
  if (method === "GET" && /^\/openai\/v1\/models\/?(\?.*)?$/.test(url)) {
    json(res, 200, { data: listMockLlmModels() });
    return true;
  }

  if (method === "POST" && /^\/openai\/v1\/embeddings\/?$/.test(url)) {
    const inputs = Array.isArray(parsedBody?.input)
      ? parsedBody.input
      : [parsedBody?.input ?? ""];
    // Honor the OpenAI `dimensions` parameter when present (the embeddings
    // client sends it for `text-embedding-3-*` models). Managed embeddings
    // don't send that parameter, so tests can set `embeddingDimensions` to the
    // managed model's fixed width. Keep the historical 4-wide default for
    // callers without either value.
    const configuredDims = Number(mockBehavior.embeddingDimensions);
    const fallbackDims =
      Number.isInteger(configuredDims) && configuredDims > 0
        ? configuredDims
        : 4;
    const requestedDims =
      Number.isInteger(parsedBody?.dimensions) && parsedBody.dimensions > 0
        ? parsedBody.dimensions
        : fallbackDims;
    const data = inputs.map((input, index) => {
      const text = String(input ?? "");
      // Keep the vector deterministic so callers that cache / compare
      // embeddings can still observe stable output: the first components keep
      // the original `[basis, basis/10, basis/100, 1]` pattern, padded to the
      // requested length.
      const basis = text.length || index + 1;
      const seed = [basis, basis / 10, basis / 100, 1];
      const embedding = Array.from(
        { length: requestedDims },
        (_, i) => seed[i] ?? 0,
      );
      return {
        object: "embedding",
        index,
        embedding,
      };
    });
    json(res, 200, {
      object: "list",
      data,
      model:
        typeof parsedBody?.model === "string" && parsedBody.model.length > 0
          ? parsedBody.model
          : "text-embedding-3-small",
      usage: {
        prompt_tokens: inputs.reduce(
          (acc, input) => acc + String(input ?? "").length,
          0,
        ),
        total_tokens: inputs.reduce(
          (acc, input) => acc + String(input ?? "").length,
          0,
        ),
      },
    });
    return true;
  }

  // (chat/completions is handled by routes/llm.mjs ahead of this route)

  // ── Composio ───────────────────────────────────────────────
  if (
    method === "GET" &&
    /^\/(?:api\/v3\/)?connected_accounts\/?(\?.*)?$/.test(url)
  ) {
    const items = parseBehaviorJson("composioDirectConnectedAccounts", []);
    json(res, 200, { items });
    return true;
  }

  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/toolkits\/?(\?.*)?$/.test(url)
  ) {
    const toolkits = parseBehaviorJson("composioToolkits", ["gmail"]);
    json(res, 200, { success: true, data: { toolkits } });
    return true;
  }

  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/connections\/?(\?.*)?$/.test(url)
  ) {
    const connections = parseBehaviorJson("composioConnections", [
      { id: "c1", toolkit: "gmail", status: "ACTIVE" },
    ]);
    // Apply per-toolkit status overrides via composioConnectionStatus_<slug>
    const overridden = connections.map((c) => {
      const statusKey = `composioConnectionStatus_${c.toolkit}`;
      const overrideStatus = mockBehavior[statusKey];
      return overrideStatus ? { ...c, status: overrideStatus } : c;
    });
    json(res, 200, { success: true, data: { connections: overridden } });
    return true;
  }

  if (
    method === "POST" &&
    /^\/agent-integrations\/composio\/authorize\/?$/.test(url)
  ) {
    const toolkit =
      typeof parsedBody?.toolkit === "string" ? parsedBody.toolkit.trim() : "";
    if (!toolkit) {
      json(res, 400, {
        success: false,
        error: "Missing required field: toolkit",
      });
      return true;
    }
    json(res, 200, {
      success: true,
      data: {
        connectUrl: `https://composio.example/${toolkit}/consent`,
        connectionId: `conn-${toolkit}-pending`,
      },
    });
    return true;
  }

  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/triggers\/available(\?.*)?$/.test(url)
  ) {
    const triggers = parseBehaviorJson("composioAvailableTriggers", [
      { slug: "GMAIL_NEW_GMAIL_MESSAGE", scope: "static" },
    ]);
    json(res, 200, { success: true, data: { triggers } });
    return true;
  }

  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/triggers(\?.*)?$/.test(url)
  ) {
    const triggers = parseBehaviorJson("composioActiveTriggers", []);
    json(res, 200, { success: true, data: { triggers } });
    return true;
  }

  if (
    method === "POST" &&
    /^\/agent-integrations\/composio\/triggers\/?$/.test(url)
  ) {
    if (mockBehavior.composioEnableFails === "1") {
      json(res, 500, { success: false, error: "Mock enable trigger failure" });
      return true;
    }
    const slug =
      typeof parsedBody?.slug === "string" ? parsedBody.slug.trim() : "";
    const connectionId =
      typeof parsedBody?.connectionId === "string"
        ? parsedBody.connectionId.trim()
        : "";
    if (!slug) {
      json(res, 400, { success: false, error: "Missing required field: slug" });
      return true;
    }
    if (!connectionId) {
      json(res, 400, {
        success: false,
        error: "Missing required field: connectionId",
      });
      return true;
    }
    const triggerId = `ti-${Date.now()}`;
    const active = parseBehaviorJson("composioActiveTriggers", []);
    active.push({
      id: triggerId,
      slug,
      toolkit: slug.split("_")[0]?.toLowerCase() ?? "",
      connectionId,
      ...(parsedBody?.triggerConfig
        ? { triggerConfig: parsedBody.triggerConfig }
        : {}),
    });
    setMockBehavior("composioActiveTriggers", JSON.stringify(active));
    json(res, 200, {
      success: true,
      data: { triggerId, slug, connectionId },
    });
    return true;
  }

  if (
    method === "DELETE" &&
    /^\/agent-integrations\/composio\/triggers\/[^/]+\/?$/.test(url)
  ) {
    let id = url.split("/").filter(Boolean).pop() ?? "";
    id = id.split("?")[0];
    if (!id) {
      json(res, 400, { success: false, error: "Missing trigger id" });
      return true;
    }
    try {
      id = decodeURIComponent(id);
    } catch {
      json(res, 400, { success: false, error: "Invalid trigger id encoding" });
      return true;
    }
    const active = parseBehaviorJson("composioActiveTriggers", []);
    const next = active.filter((t) => t.id !== id);
    const deleted = next.length !== active.length;
    if (deleted) {
      setMockBehavior("composioActiveTriggers", JSON.stringify(next));
    }
    json(res, 200, { success: true, data: { deleted } });
    return true;
  }

  // Composio gap fills.
  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/github\/repos\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: { repos: [] } });
    return true;
  }

  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/tools\/?(\?.*)?$/.test(url)
  ) {
    // Parse toolkits from the query string.
    const qs = url.includes("?")
      ? new URLSearchParams(url.split("?")[1])
      : new URLSearchParams();
    const toolkitsParam = qs.get("toolkits") ?? "";
    const requestedToolkits = toolkitsParam
      ? toolkitsParam
          .split(",")
          .map((t) => t.trim().toLowerCase())
          .filter(Boolean)
      : [];
    // Allow tests to inject the tool list through the composioTools knob.
    // Filter by toolkits when requested and the knob returns names such as
    // "GITHUB_*".
    let tools = parseBehaviorJson("composioTools", []);
    if (requestedToolkits.length > 0 && tools.length > 0) {
      tools = tools.filter((t) => {
        const name = (t?.function?.name ?? t?.name ?? "").toUpperCase();
        return requestedToolkits.some((tk) =>
          name.startsWith(tk.toUpperCase() + "_"),
        );
      });
    }

    json(res, 200, { success: true, data: { tools } });
    return true;
  }

  if (
    method === "POST" &&
    /^\/agent-integrations\/composio\/execute\/?$/.test(url)
  ) {
    const action =
      typeof parsedBody?.action === "string"
        ? parsedBody.action
        : typeof parsedBody?.tool === "string"
          ? parsedBody.tool
          : "";
    // composioExecuteFails → inject error response
    // Knob values: '400' or '1' → HTTP 400; '500' → HTTP 500
    if (
      mockBehavior.composioExecuteFails === "400" ||
      mockBehavior.composioExecuteFails === "1"
    ) {
      json(res, 400, {
        success: false,
        error: "Mock execute failure",
        data: { successful: false, data: null, error: "Mock execute failure" },
      });
      return true;
    }
    if (mockBehavior.composioExecuteFails === "500") {
      json(res, 500, {
        success: false,
        error: "Mock execute server error",
        data: {
          successful: false,
          data: null,
          error: "Mock execute server error",
        },
      });
      return true;
    }
    // Per-action override: composioExecuteResponse_<ACTION>
    const actionKey = `composioExecuteResponse_${action}`;
    if (mockBehavior[actionKey]) {
      let overrideData;
      try {
        overrideData = JSON.parse(mockBehavior[actionKey]);
      } catch {
        overrideData = { ok: true };
      }
      json(res, 200, {
        success: true,
        data: { successful: true, data: overrideData, error: null },
      });
      return true;
    }
    const data =
      action === "GMAIL_FETCH_EMAILS"
        ? {
            messages: [
              {
                id: "e2e-gmail-message-1",
                snippet:
                  "Welcome to OpenHuman. No profile link is required for this run.",
              },
            ],
          }
        : { ok: true };
    json(res, 200, {
      success: true,
      data: { successful: true, data, error: null },
    });
    return true;
  }

  // ── Composio connection delete ─────────────────────────────
  if (
    method === "DELETE" &&
    /^\/agent-integrations\/composio\/connections\/[^/]+\/?$/.test(url)
  ) {
    if (mockBehavior.composioDeleteFails === "400") {
      json(res, 400, {
        success: false,
        error: "Mock connection delete failure",
      });
      return true;
    }
    if (
      mockBehavior.composioDeleteFails === "500" ||
      mockBehavior.composioDeleteFails === "1"
    ) {
      json(res, 500, {
        success: false,
        error: "Mock connection delete failure",
      });
      return true;
    }
    let connId = url.split("/").filter(Boolean).pop() ?? "";
    connId = connId.split("?")[0];
    try {
      connId = decodeURIComponent(connId);
    } catch {
      json(res, 400, {
        success: false,
        error: "Invalid connection id encoding",
      });
      return true;
    }
    // Remove the connection from the seeded list if present
    const conns = parseBehaviorJson("composioConnections", [
      { id: "c1", toolkit: "gmail", status: "ACTIVE" },
    ]);
    const next = conns.filter((c) => c.id !== connId);
    const deleted = next.length !== conns.length;
    setMockBehavior("composioConnections", JSON.stringify(next));
    json(res, 200, { success: true, data: { deleted } });
    return true;
  }

  // ── Parallel search ────────────────────────────────────────
  if (
    method === "POST" &&
    /^\/agent-integrations\/parallel\/search\/?$/.test(url)
  ) {
    const objective =
      typeof parsedBody?.objective === "string" &&
      parsedBody.objective.trim().length > 0
        ? parsedBody.objective.trim()
        : "unknown objective";
    const queries = Array.isArray(parsedBody?.searchQueries)
      ? parsedBody.searchQueries
          .map((query) => String(query ?? "").trim())
          .filter(Boolean)
      : [];
    const effectiveQueries = queries.length > 0 ? queries : [objective];
    const results = effectiveQueries.map((query, index) => ({
      url: `https://search.example.com/${index}`,
      title: `Result for ${query}`,
      publish_date: "2026-05-16",
      excerpts: [`Objective: ${objective}; query: ${query}`],
    }));
    json(res, 200, {
      success: true,
      data: {
        searchId: `search-${effectiveQueries.length}`,
        results,
        costUsd: 0.02,
      },
    });
    return true;
  }

  // ── Exa (managed) ──────────────────────────────────────────
  // Mirrors backend `/agent-integrations/exa/*`: search takes
  // `{objective, searchQueries}`; contents/answer/findSimilar proxy Exa's
  // own REST bodies. `mockBehavior.exaInsufficientBalance = "1"` answers
  // the backend's insufficient-credits 400.
  if (method === "POST" && /^\/agent-integrations\/exa\//.test(url)) {
    if (mockBehavior.exaInsufficientBalance === "1") {
      json(res, 400, {
        success: false,
        error: "Insufficient balance",
        errorCode: "USER_INSUFFICIENT_CREDITS",
      });
      return true;
    }
    if (/^\/agent-integrations\/exa\/search\/?$/.test(url)) {
      const objective =
        typeof parsedBody?.objective === "string"
          ? parsedBody.objective.trim()
          : "";
      const queries = Array.isArray(parsedBody?.searchQueries)
        ? parsedBody.searchQueries
            .map((query) => String(query ?? "").trim())
            .filter(Boolean)
        : [];
      if (!objective || queries.length === 0 || "mode" in (parsedBody ?? {})) {
        json(res, 400, { success: false, error: "Invalid Exa search request" });
        return true;
      }
      json(res, 200, {
        success: true,
        data: {
          searchId: `exa-search-${queries.length}`,
          results: queries.map((query, index) => ({
            url: `https://exa.example.com/${index}`,
            title: `Exa result for ${query}`,
            publish_date: "2026-09-01",
            excerpts: [`Objective: ${objective}; query: ${query}`],
          })),
          costUsd: 0.01,
        },
      });
      return true;
    }
    if (/^\/agent-integrations\/exa\/contents\/?$/.test(url)) {
      const urls = Array.isArray(parsedBody?.urls)
        ? parsedBody.urls
        : Array.isArray(parsedBody?.ids)
          ? parsedBody.ids
          : [];
      json(res, 200, {
        success: true,
        data: {
          requestId: "exa-contents",
          results: urls.map((pageUrl) => ({
            id: String(pageUrl),
            url: String(pageUrl),
            title: `Contents of ${pageUrl}`,
            text: `Mock page text for ${pageUrl}`,
          })),
          costDollars: { total: 0.001 },
        },
      });
      return true;
    }
    if (/^\/agent-integrations\/exa\/answer\/?$/.test(url)) {
      const query = String(parsedBody?.query ?? "").trim();
      json(res, 200, {
        success: true,
        data: {
          requestId: "exa-answer",
          answer: `Mock Exa answer for ${query}`,
          citations: [
            {
              id: "c0",
              url: "https://exa.example.com/answer",
              title: "Exa source",
            },
          ],
          costDollars: { total: 0.005 },
        },
      });
      return true;
    }
    if (/^\/agent-integrations\/exa\/findSimilar\/?$/.test(url)) {
      json(res, 200, {
        success: true,
        data: {
          requestId: "exa-similar",
          results: [
            {
              id: "s0",
              url: "https://exa.example.com/similar",
              title: "Similar page",
            },
          ],
          costDollars: { total: 0.005 },
        },
      });
      return true;
    }
  }

  // ── Gemini grounded generate-content (managed) ─────────────
  {
    const match = url.match(
      /^\/agent-integrations\/gemini\/models\/([^/?]+)\/generate-content\/?$/,
    );
    if (method === "POST" && match) {
      const model = decodeURIComponent(match[1]);
      const prompt = String(
        parsedBody?.contents?.[0]?.parts?.[0]?.text ?? "",
      ).trim();
      json(res, 200, {
        success: true,
        data: {
          modelVersion: model,
          responseId: "gemini-mock",
          candidates: [
            {
              content: {
                role: "model",
                parts: [{ text: `Mock grounded answer for ${prompt}` }],
              },
              groundingMetadata: {
                webSearchQueries: [prompt],
                groundingChunks: [
                  {
                    web: {
                      uri: "https://gemini.example.com/source",
                      title: "Grounding source",
                    },
                  },
                ],
                groundingSupports: [
                  {
                    segment: { startIndex: 0, endIndex: 10 },
                    groundingChunkIndices: [0],
                  },
                ],
              },
            },
          ],
          usageMetadata: { promptTokenCount: 10, candidatesTokenCount: 20 },
          costUsd: 0.002,
        },
      });
      return true;
    }
  }

  // ── TinyFish (managed) ─────────────────────────────────────
  if (
    method === "POST" &&
    /^\/agent-integrations\/tinyfish\/(search|fetch)\/?$/.test(url)
  ) {
    const isFetch = /\/fetch\/?$/.test(url);
    json(res, 200, {
      success: true,
      data: isFetch
        ? {
            results: (parsedBody?.urls ?? []).map((pageUrl) => ({
              url: String(pageUrl),
              title: `Fetched ${pageUrl}`,
              text: `Mock fetched text for ${pageUrl}`,
            })),
          }
        : {
            results: [
              {
                url: "https://tinyfish.example.com/0",
                title: `TinyFish result for ${String(parsedBody?.query ?? "")}`,
                snippet: "Mock TinyFish snippet",
              },
            ],
          },
    });
    return true;
  }

  // ── Composio user-scopes ───────────────────────────────────
  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/user-scopes\/?(\?.*)?$/.test(url)
  ) {
    const scopes = parseBehaviorJson("composioUserScopes", {
      read: true,
      write: true,
      admin: false,
    });
    json(res, 200, { success: true, data: scopes });
    return true;
  }

  if (
    method === "POST" &&
    /^\/agent-integrations\/composio\/user-scopes\/?$/.test(url)
  ) {
    // Echo back the posted preferences and persist them as the new scopes
    const pref = parsedBody ?? {};
    setMockBehavior("composioUserScopes", JSON.stringify(pref));
    json(res, 200, { success: true, data: pref });
    return true;
  }

  return false;
}
