import { json } from "../http.mjs";
import { behavior, parseBehaviorJson, setMockBehavior } from "../state.mjs";

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
    json(res, 200, { data: [{ id: "e2e-mock-model", object: "model" }] });
    return true;
  }

  if (method === "POST" && /^\/openai\/v1\/chat\/completions\/?$/.test(url)) {
    json(res, 200, {
      choices: [
        {
          message: { role: "assistant", content: "Hello from e2e mock agent" },
        },
      ],
    });
    return true;
  }

  // ── Composio ───────────────────────────────────────────────
  if (
    method === "GET" &&
    /^\/agent-integrations\/composio\/connections\/?(\?.*)?$/.test(url)
  ) {
    const connections = parseBehaviorJson("composioConnections", [
      { id: "c1", toolkit: "gmail", status: "ACTIVE" },
    ]);
    json(res, 200, { success: true, data: { connections } });
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
    json(res, 200, { success: true, data: { tools: [] } });
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
        data: { id: runId, status: "SUCCEEDED", finishedAt: new Date().toISOString() },
      });
    }
    return true;
  }

  return false;
}
