import { json } from "../http.mjs";
import { behavior, parseBehaviorJson, setMockBehavior } from "../state.mjs";
import { listMockLlmModels } from "./llm/shared.mjs";

export function handleIntegrations(ctx) {
  const { method, url, parsedBody, res } = ctx;
  const mockBehavior = behavior();

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
    const data = inputs.map((input, index) => {
      const text = String(input ?? "");
      // Keep the vector tiny but deterministic so callers that cache /
      // compare embeddings can still observe stable output.
      const basis = text.length || index + 1;
      return {
        object: "embedding",
        index,
        embedding: [basis, basis / 10, basis / 100, 1],
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
    // Parse toolkits and tags from the query string.
    const qs = url.includes("?") ? new URLSearchParams(url.split("?")[1]) : new URLSearchParams();
    const toolkitsParam = qs.get("toolkits") ?? "";
    const tagsParam = qs.get("tags") ?? "";
    const requestedToolkits = toolkitsParam ? toolkitsParam.split(",").map(t => t.trim().toLowerCase()).filter(Boolean) : [];
    const requestedTags = tagsParam ? tagsParam.split(",").map(t => t.trim().toLowerCase()).filter(Boolean) : [];

    // Allow tests to inject per-tag tool lists via
    //   composioToolsByTag_<tag>  (e.g. "composioToolsByTag_stars")
    // or a catch-all composioTools knob (array of tool objects).
    // Falls back to [] when no knob is set.
    let tools = [];

    // Mirror the Rust gate: tags are only honoured when no toolkit filter is
    // active or the toolkit list includes GitHub.
    const hasGithubToolkit =
      requestedToolkits.length === 0 || requestedToolkits.includes("github");
    const effectiveTags = hasGithubToolkit ? requestedTags : [];

    if (effectiveTags.length > 0) {
      // OR semantics: union across all requested tags.
      const seen = new Set();
      for (const tag of effectiveTags) {
        const knobKey = `composioToolsByTag_${tag}`;
        const tagTools = parseBehaviorJson(knobKey, null);
        if (Array.isArray(tagTools)) {
          for (const t of tagTools) {
            const name = t?.function?.name ?? t?.name ?? JSON.stringify(t);
            if (!seen.has(name)) {
              seen.add(name);
              tools.push(t);
            }
          }
        }
      }
    } else {
      tools = parseBehaviorJson("composioTools", []);
      // Filter by toolkits when requested and the knob returns a list with a
      // "function.name" slug we can match (e.g. "GITHUB_*").
      if (requestedToolkits.length > 0 && tools.length > 0) {
        tools = tools.filter(t => {
          const name = (t?.function?.name ?? t?.name ?? "").toUpperCase();
          return requestedToolkits.some(tk => name.startsWith(tk.toUpperCase() + "_"));
        });
      }
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

  // ── Composio sync ──────────────────────────────────────────
  if (
    method === "POST" &&
    /^\/agent-integrations\/composio\/sync\/?$/.test(url)
  ) {
    if (mockBehavior.composioSyncFails === "400") {
      json(res, 400, { success: false, error: "Mock sync failure" });
      return true;
    }
    if (
      mockBehavior.composioSyncFails === "500" ||
      mockBehavior.composioSyncFails === "1"
    ) {
      json(res, 500, { success: false, error: "Mock sync failure" });
      return true;
    }
    json(res, 200, { success: true, data: { items_synced: 3 } });
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

  // ── Apify ──────────────────────────────────────────────────
  // Gap fill — minimal stubs for run polling.
  const apifyMatch = url.match(
    /^\/agent-integrations\/apify\/runs\/([^/?]+)(\/results)?\/?(\?.*)?$/,
  );
  if (apifyMatch && method === "GET") {
    const [, runId, isResults] = apifyMatch;
    if (isResults) {
      json(res, 200, { success: true, data: { items: [] } });
    } else {
      json(res, 200, {
        success: true,
        data: {
          id: runId,
          status: "SUCCEEDED",
          finishedAt: new Date().toISOString(),
        },
      });
    }
    return true;
  }

  return false;
}
