import crypto from "node:crypto";
import http from "node:http";

const DEFAULT_PORT = 18473;
const MOCK_JWT = "e2e-mock-jwt-token";

let requestLog = [];
let mockBehavior = {};
let server = null;
const openSockets = new Set();
let mockTunnels = [];

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PUT, PATCH, DELETE, OPTIONS",
  "Access-Control-Allow-Headers":
    "Content-Type, Authorization, x-device-fingerprint",
  "Access-Control-Max-Age": "86400",
};

function setCors(res) {
  for (const [key, value] of Object.entries(CORS_HEADERS)) {
    res.setHeader(key, value);
  }
}

function json(res, status, body) {
  setCors(res);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

function html(res, status, body) {
  setCors(res);
  res.writeHead(status, { "Content-Type": "text/html; charset=utf-8" });
  res.end(body);
}

function requestOrigin(req) {
  const host = req.headers.host || "127.0.0.1:18473";
  return `http://${host}`;
}

function getMockUser() {
  return {
    _id: "user-123",
    telegramId: 12345678,
    hasAccess: true,
    magicWord: "alpha",
    firstName: "Test",
    lastName: "User",
    username: "testuser",
    role: "user",
    activeTeamId: "team-1",
    referral: {},
    subscription: { hasActiveSubscription: false, plan: "FREE" },
    settings: {
      dailySummariesEnabled: false,
      dailySummaryChatIds: [],
      autoCompleteEnabled: false,
      autoCompleteVisibility: "always",
      autoCompleteWhitelistChatIds: [],
      autoCompleteBlacklistChatIds: [],
    },
    usage: {
      cycleBudgetUsd: 10,
      remainingUsd: 10,
      spentThisCycleUsd: 0,
      spentTodayUsd: 0,
      cycleStartDate: new Date().toISOString(),
    },
    autoDeleteTelegramMessagesAfterDays: 30,
    autoDeleteThreadsAfterDays: 30,
  };
}

function getMockTeam() {
  const plan = mockBehavior.plan || "FREE";
  const isActive = mockBehavior.planActive === "true";
  const expiry = mockBehavior.planExpiry || null;
  return {
    team: {
      _id: "team-1",
      name: "Personal",
      slug: "personal",
      createdBy: "test-user-123",
      isPersonal: true,
      maxMembers: 1,
      subscription: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
      },
      usage: {
        dailyTokenLimit: 1000,
        remainingTokens: 1000,
        activeSessionCount: 0,
      },
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    },
    role: "ADMIN",
  };
}

function getRequestLog() {
  return [...requestLog];
}

function clearRequestLog() {
  requestLog = [];
}

function resetMockTunnels() {
  mockTunnels = [];
}

function setMockBehavior(key, value) {
  mockBehavior[key] = String(value);
}

function setMockBehaviors(behavior, mode = "merge") {
  if (mode === "replace") {
    mockBehavior = {};
  }
  for (const [key, value] of Object.entries(behavior || {})) {
    mockBehavior[key] = String(value);
  }
}

function resetMockBehavior() {
  mockBehavior = {};
}

function getMockBehavior() {
  return { ...mockBehavior };
}

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString()));
  });
}

function tryParseJson(raw) {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function getDelayMs(key) {
  const value = Number(mockBehavior[key] || 0);
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function createMockTunnel(payload = {}) {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    uuid: crypto.randomUUID(),
    name: String(payload.name || "Mock Tunnel").trim(),
    description: String(payload.description || "").trim(),
    isActive: payload.isActive ?? true,
    createdAt: now,
    updatedAt: now,
  };
}

async function handleRequest(req, res) {
  const method = req.method ?? "GET";
  const url = req.url ?? "/";
  const body = await readBody(req);
  const parsedBody = tryParseJson(body);
  const origin = requestOrigin(req);

  requestLog.push({ method, url, body, timestamp: Date.now() });

  if (method === "OPTIONS") {
    setCors(res);
    res.writeHead(204);
    res.end();
    return;
  }

  if (method === "GET" && /^\/__admin\/health\/?$/.test(url)) {
    json(res, 200, { ok: true, port: server?.address()?.port ?? null });
    return;
  }
  if (method === "GET" && /^\/__admin\/requests\/?$/.test(url)) {
    json(res, 200, { success: true, data: getRequestLog() });
    return;
  }
  if (method === "GET" && /^\/__admin\/behavior\/?$/.test(url)) {
    json(res, 200, { success: true, data: getMockBehavior() });
    return;
  }
  if (method === "POST" && /^\/__admin\/reset\/?$/.test(url)) {
    const keepBehavior = parsedBody?.keepBehavior === true;
    const keepRequests = parsedBody?.keepRequests === true;
    if (!keepBehavior) resetMockBehavior();
    if (!keepRequests) clearRequestLog();
    resetMockTunnels();
    json(res, 200, {
      success: true,
      data: {
        behavior: getMockBehavior(),
        requestCount: getRequestLog().length,
      },
    });
    return;
  }
  if (method === "POST" && /^\/__admin\/behavior\/?$/.test(url)) {
    if (parsedBody?.behavior && typeof parsedBody.behavior === "object") {
      setMockBehaviors(parsedBody.behavior, parsedBody.mode);
    } else if (parsedBody?.key) {
      setMockBehavior(parsedBody.key, parsedBody.value ?? "");
    }
    json(res, 200, { success: true, data: getMockBehavior() });
    return;
  }

  if (url.startsWith("/socket.io/")) {
    const eioOpen = JSON.stringify({
      sid: "mock-sid-" + Date.now(),
      upgrades: ["websocket"],
      pingInterval: 25000,
      pingTimeout: 20000,
    });
    const packet = `${eioOpen.length + 1}:0${eioOpen}`;
    setCors(res);
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end(packet);
    return;
  }

  if (
    method === "POST" &&
    /^\/telegram\/login-tokens\/[^/]+\/consume\/?$/.test(url)
  ) {
    if (mockBehavior.token === "expired") {
      json(res, 401, { success: false, error: "Token expired or invalid" });
      return;
    }
    if (mockBehavior.token === "invalid") {
      json(res, 401, { success: false, error: "Invalid token" });
      return;
    }
    const jwt = mockBehavior.jwt ? `${MOCK_JWT}-${mockBehavior.jwt}` : MOCK_JWT;
    json(res, 200, { success: true, data: { jwtToken: jwt } });
    return;
  }

  if (method === "POST" && /^\/auth\/desktop-exchange\/?$/.test(url)) {
    json(res, 200, {
      sessionToken: "mock-session-token",
      user: { id: "user-123", firstName: "Test", username: "testuser" },
    });
    return;
  }

  if (
    method === "GET" &&
    (/^\/telegram\/me\/?(\?.*)?$/.test(url) || /^\/auth\/me\/?(\?.*)?$/.test(url))
  ) {
    const delayMs = getDelayMs("telegramMeDelayMs");
    if (delayMs > 0) {
      await sleep(delayMs);
    }
    if (mockBehavior.telegramMeStatus) {
      const status = Number(mockBehavior.telegramMeStatus) || 500;
      json(res, status, {
        success: false,
        error: mockBehavior.telegramMeError || "Mock telegram/me failure",
      });
      return;
    }
    if (mockBehavior.session === "revoked") {
      json(res, 401, { success: false, error: "Unauthorized" });
      return;
    }
    json(res, 200, { success: true, data: getMockUser() });
    return;
  }

  if (method === "GET" && /^\/settings\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { _id: "e2e-user-1", username: "e2e" },
    });
    return;
  }

  if (method === "GET" && /^\/teams\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [getMockTeam()] });
    return;
  }

  if (method === "GET" && /^\/teams\/me\/usage\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        cycleBudgetUsd: 10,
        remainingUsd: 10,
        cycleLimit5hr: 0,
        cycleLimit7day: 0,
        fiveHourCapUsd: 5,
        fiveHourResetsAt: null,
        cycleStartDate: new Date().toISOString(),
        cycleEndsAt: new Date(Date.now() + 7 * 86400000).toISOString(),
        fiveHourSpendUsd: 0,
        bypassCycleLimit: false,
      },
    });
    return;
  }

  if (method === "GET" && /^\/auth\/integrations\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return;
  }

  if (method === "POST" && /^\/webhooks\/core\/?$/.test(url)) {
    const tunnel = createMockTunnel(parsedBody || {});
    mockTunnels.unshift(tunnel);
    json(res, 200, { success: true, data: tunnel });
    return;
  }

  if (method === "GET" && /^\/webhooks\/core\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: mockTunnels });
    return;
  }

  if (method === "GET" && /^\/webhooks\/core\/bandwidth\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: { remainingBudgetUsd: 10 } });
    return;
  }

  const webhookCoreMatch = url.match(/^\/webhooks\/core\/([^/?]+)\/?(\?.*)?$/);
  if (webhookCoreMatch) {
    const [, tunnelId] = webhookCoreMatch;
    const tunnelIndex = mockTunnels.findIndex((entry) => entry.id === tunnelId);
    const tunnel = tunnelIndex >= 0 ? mockTunnels[tunnelIndex] : null;

    if (!tunnel) {
      json(res, 404, { success: false, error: "Tunnel not found" });
      return;
    }

    if (method === "GET") {
      json(res, 200, { success: true, data: tunnel });
      return;
    }

    if (method === "PATCH") {
      const updated = {
        ...tunnel,
        ...(parsedBody || {}),
        updatedAt: new Date().toISOString(),
      };
      mockTunnels[tunnelIndex] = updated;
      json(res, 200, { success: true, data: updated });
      return;
    }

    if (method === "DELETE") {
      mockTunnels.splice(tunnelIndex, 1);
      json(res, 200, { success: true, data: tunnel });
      return;
    }
  }

  // --- Payments / Credits / Billing ---

  if (
    method === "GET" &&
    /^\/payments\/credits\/balance\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, {
      success: true,
      data: { balanceUsd: 10, topUpBalanceUsd: 0, topUpBaselineUsd: 0 },
    });
    return;
  }

  if (
    method === "GET" &&
    (/^\/payments\/plan\/?(\?.*)?$/.test(url) ||
      /^\/payments\/stripe\/currentPlan\/?(\?.*)?$/.test(url))
  ) {
    const plan = mockBehavior.plan || "FREE";
    const isActive = mockBehavior.planActive === "true";
    const periodEnd = new Date(Date.now() + 30 * 86400000).toISOString();
    json(res, 200, {
      success: true,
      data: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: isActive ? periodEnd : null,
        subscription: isActive
          ? { id: "sub_mock_1", status: "active", currentPeriodEnd: periodEnd, quantity: 1 }
          : null,
        monthlyBudgetUsd: 0,
        weeklyBudgetUsd: 0,
        fiveHourCapUsd: 0,
      },
    });
    return;
  }

  if (
    method === "POST" &&
    (/^\/payments\/stripe\/checkout\/?$/.test(url) ||
      /^\/payments\/stripe\/purchasePlan\/?$/.test(url))
  ) {
    if (mockBehavior.purchaseError === "true") {
      json(res, 500, { success: false, error: "Payment service unavailable" });
      return;
    }
    json(res, 200, {
      success: true,
      data: {
        sessionId: "cs_mock_" + Date.now(),
        // Return null checkoutUrl so the app doesn't navigate the WebView away.
        // The test verifies the API call was made, not the redirect.
        checkoutUrl: null,
      },
    });
    return;
  }

  if (method === "POST" && /^\/payments\/stripe\/portal\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { portalUrl: "https://billing.stripe.com/mock-portal" },
    });
    return;
  }

  if (method === "POST" && /^\/payments\/coinbase\/charge\/?$/.test(url)) {
    if (mockBehavior.coinbaseError === "true") {
      json(res, 500, { success: false, error: "Coinbase service unavailable" });
      return;
    }
    json(res, 200, {
      success: true,
      data: {
        gatewayTransactionId: "charge_mock_" + Date.now(),
        hostedUrl: "https://commerce.coinbase.com/mock-charge",
        status: "NEW",
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return;
  }

  if (method === "POST" && /^\/payments\/purchase\/?$/.test(url)) {
    const plan = parsedBody?.plan || mockBehavior.plan || "BASIC";
    json(res, 200, {
      success: true,
      data: {
        sessionId: "cs_mock_" + Date.now(),
        url: "https://checkout.stripe.com/mock-purchase",
        plan,
      },
    });
    return;
  }

  if (
    method === "GET" &&
    /^\/payments\/credits\/auto-recharge\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, {
      success: true,
      data: {
        enabled: false,
        thresholdUsd: 5,
        rechargeAmountUsd: 10,
        weeklyLimitUsd: 50,
        spentThisWeekUsd: 0,
        weekStartDate: new Date().toISOString(),
        inFlight: false,
        hasSavedPaymentMethod: false,
        lastTriggeredAt: null,
        lastRechargeAt: null,
      },
    });
    return;
  }

  if (method === "GET" && /^\/payments\/cards\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: { cards: [], defaultCardId: null } });
    return;
  }

  if (
    method === "GET" &&
    /^\/payments\/credits\/auto-recharge\/cards\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: { cards: [], defaultCardId: null } });
    return;
  }

  if (method === "GET" && /^\/openai\/v1\/models\/?(\?.*)?$/.test(url)) {
    json(res, 200, { data: [{ id: "e2e-mock-model", object: "model" }] });
    return;
  }

  if (method === "POST" && /^\/openai\/v1\/chat\/completions\/?$/.test(url)) {
    // When chatToolCalls behavior is set, return a tool_calls response on the
    // first completion request, then a normal text response on subsequent ones.
    // Format: "toolName|argumentsJson" e.g. "e2e-runtime__echo|{\"message\":\"hi\"}"
    if (mockBehavior.chatToolCalls) {
      const spec = mockBehavior.chatToolCalls;
      // Clear after first use so the second call returns normal text
      delete mockBehavior.chatToolCalls;
      const pipeIdx = spec.indexOf("|");
      const toolName = pipeIdx > 0 ? spec.slice(0, pipeIdx) : spec;
      const toolArgs = pipeIdx > 0 ? spec.slice(pipeIdx + 1) : "{}";
      json(res, 200, {
        choices: [
          {
            message: {
              role: "assistant",
              content: null,
              tool_calls: [
                {
                  id: "call_e2e_" + Date.now(),
                  type: "function",
                  function: { name: toolName, arguments: toolArgs },
                },
              ],
            },
            finish_reason: "tool_calls",
          },
        ],
      });
      return;
    }
    json(res, 200, {
      choices: [
        {
          message: { role: "assistant", content: "Hello from e2e mock agent" },
        },
      ],
    });
    return;
  }

  if (method === "GET" && /^\/auth\/[^/]+\/login\/?(\?.*)?$/.test(url)) {
    const redirectUrl = `${origin}/mock-oauth`;
    if (url.includes("responseType=json")) {
      json(res, 200, { success: true, data: { oauthUrl: redirectUrl } });
      return;
    }
    setCors(res);
    res.writeHead(302, { Location: redirectUrl });
    res.end();
    return;
  }

  if (method === "GET" && /^\/auth\/telegram\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.telegramDuplicate === "true") {
      json(res, 409, {
        success: false,
        error: "Telegram account already linked to another user",
      });
      return;
    }
    json(res, 200, {
      success: true,
      oauthUrl: `${origin}/mock-telegram-oauth`,
      state: "mock-telegram-state",
    });
    return;
  }

  if (method === "GET" && /^\/auth\/notion\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.notionTokenRevoked === "true") {
      json(res, 401, { success: false, error: "OAuth token has been revoked" });
      return;
    }
    const workspace = mockBehavior.notionWorkspace || "Test User's Workspace";
    json(res, 200, {
      success: true,
      oauthUrl: `${origin}/mock-notion-oauth`,
      state: "mock-notion-state",
      workspace,
    });
    return;
  }

  if (method === "GET" && /^\/auth\/google\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.gmailTokenRevoked === "true") {
      json(res, 401, { success: false, error: "OAuth token has been revoked" });
      return;
    }
    if (mockBehavior.gmailTokenExpired === "true") {
      json(res, 401, { success: false, error: "OAuth token has expired" });
      return;
    }
    json(res, 200, {
      success: true,
      oauthUrl: `${origin}/mock-google-oauth`,
      state: "mock-google-state",
    });
    return;
  }

  if (method === "POST" && /^\/telegram\/command\/?$/.test(url)) {
    if (mockBehavior.telegramUnauthorized === "true") {
      json(res, 403, {
        success: false,
        error: "Unauthorized: insufficient permissions",
      });
      return;
    }
    if (mockBehavior.telegramCommandError === "true") {
      json(res, 400, { success: false, error: "Invalid command format" });
      return;
    }
    json(res, 200, {
      success: true,
      data: { result: "Command executed successfully" },
    });
    return;
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
    return;
  }

  if (method === "POST" && /^\/telegram\/webhook\/configure\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        webhookUrl: "https://api.example.com/webhook/telegram",
        active: true,
      },
    });
    return;
  }

  if (method === "POST" && /^\/telegram\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return;
  }

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
    return;
  }

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
    return;
  }

  if (method === "POST" && /^\/gmail\/disconnect\/?$/.test(url)) {
    json(res, 200, { success: true, data: { disconnected: true } });
    return;
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
    return;
  }

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
    return;
  }

  if (method === "POST" && /^\/invite\/redeem\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { message: "Invite code redeemed successfully" },
    });
    return;
  }
  if (method === "GET" && /^\/invite\/my-codes\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return;
  }
  if (method === "GET" && /^\/invite\/status/.test(url)) {
    json(res, 200, { success: true, data: { valid: true } });
    return;
  }

  if (method === "GET" && /^\/referral\/stats\/?(\?.*)?$/.test(url)) {
    const origin = requestOrigin(req);
    json(res, 200, {
      success: true,
      data: {
        referralCode: "MOCKREF1",
        referralLink: `${origin}/#/rewards?ref=MOCKREF1`,
        rewardRateBps: 2000,
        totals: {
          totalRewardUsd: 4.2,
          pendingCount: 1,
          convertedCount: 2,
        },
        referrals: [
          {
            id: "ref-row-1",
            referredUserId: "user-456",
            status: "pending",
            createdAt: new Date(Date.now() - 86400000).toISOString(),
          },
          {
            id: "ref-row-2",
            referredUserId: "user-789",
            status: "converted",
            createdAt: new Date(Date.now() - 172800000).toISOString(),
            convertedAt: new Date().toISOString(),
            rewardUsd: 2.1,
          },
        ],
        appliedReferralCode: null,
        canApplyReferral: true,
      },
    });
    return;
  }

  if (method === "POST" && /^\/referral\/apply\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { ok: true, message: "Referral applied" },
    });
    return;
  }
  if (
    method === "POST" &&
    /^\/telegram\/settings\/onboarding-complete\/?$/.test(url)
  ) {
    json(res, 200, { success: true, data: {} });
    return;
  }
  if (method === "POST" && /^\/settings\/onboarding-complete\/?$/.test(url)) {
    json(res, 200, { success: true, data: {} });
    return;
  }

  // currentPlan is handled by the earlier consolidated handler.
  if (method === "GET" && /^\/billing\/current-plan\/?(\?.*)?$/.test(url)) {
    const plan = mockBehavior.plan || "FREE";
    const isActive = mockBehavior.planActive === "true";
    const expiry = mockBehavior.planExpiry || null;
    json(res, 200, {
      success: true,
      data: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
        subscription: isActive
          ? {
              id: "sub_mock_123",
              status: "active",
              currentPeriodEnd:
                expiry || new Date(Date.now() + 30 * 86400000).toISOString(),
            }
          : null,
      },
    });
    return;
  }

  // purchasePlan, portal, and coinbase/charge are handled by the earlier
  // consolidated handlers (with mockBehavior checks). Only the coinbase
  // charge-status polling endpoint remains here.

  if (
    method === "GET" &&
    /^\/payments\/coinbase\/charge\/[^/]+\/?(\?.*)?$/.test(url)
  ) {
    const status = mockBehavior.cryptoStatus || "NEW";
    json(res, 200, {
      success: true,
      data: {
        status,
        payment: {
          status,
          amountPaid:
            status === "UNDERPAID"
              ? "150.00"
              : status === "OVERPAID"
                ? "350.00"
                : "250.00",
          amountExpected: "250.00",
          currency: "USDC",
          underpaidAmount: mockBehavior.cryptoUnderpaidAmount || "0",
          overpaidAmount: mockBehavior.cryptoOverpaidAmount || "0",
        },
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
    });
    return;
  }

  if (
    method === "GET" &&
    /^\/mock-(telegram|notion|google)-oauth\/?(\?.*)?$/.test(url)
  ) {
    html(res, 200, "<html><body><h1>Mock OAuth</h1></body></html>");
    return;
  }
  if (method === "GET" && /^\/mock-oauth\/?(\?.*)?$/.test(url)) {
    html(res, 200, "<html><body><h1>Mock OAuth Redirect</h1></body></html>");
    return;
  }

  // Catch-all: fail fast so tests notice missing mock endpoints.
  console.log(`[MockServer] UNHANDLED ${method} ${url}`);
  json(res, 404, {
    success: false,
    error: `Mock server: no handler for ${method} ${url}`,
  });
}

function handleSocketIOMessage(socket, text, sid) {
  if (text === "2") {
    sendWsText(socket, "3");
    return;
  }
  if (text.startsWith("40")) {
    sendWsText(socket, `40{"sid":"${sid}"}`);
  }
}

function sendWsText(socket, text) {
  sendWsFrame(socket, 0x01, Buffer.from(text, "utf-8"));
}

function sendWsFrame(socket, opcode, payload) {
  if (socket.destroyed) return;

  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.alloc(2);
    header[0] = 0x80 | opcode;
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  try {
    socket.write(header);
    socket.write(payload);
  } catch {
    // noop
  }
}

function handleWebSocketUpgrade(req, socket) {
  if (!req.url?.startsWith("/socket.io/")) {
    socket.destroy();
    return;
  }
  const key = req.headers["sec-websocket-key"];
  if (!key) {
    socket.destroy();
    return;
  }
  const acceptKey = crypto
    .createHash("sha1")
    .update(key + "258EAFA5-E914-47DA-95CA-5AB5DC085B11")
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${acceptKey}\r\n` +
      "\r\n",
  );

  const mockSid = "mock-ws-" + Date.now();
  const eioOpen = JSON.stringify({
    sid: mockSid,
    upgrades: [],
    pingInterval: 25000,
    pingTimeout: 60000,
    maxPayload: 1000000,
  });
  sendWsText(socket, `0${eioOpen}`);

  let buffer = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= 2) {
      const firstByte = buffer[0];
      const opcode = firstByte & 0x0f;
      const secondByte = buffer[1];
      const masked = (secondByte & 0x80) !== 0;
      let payloadLen = secondByte & 0x7f;
      let offset = 2;

      if (payloadLen === 126) {
        if (buffer.length < 4) return;
        payloadLen = buffer.readUInt16BE(2);
        offset = 4;
      } else if (payloadLen === 127) {
        if (buffer.length < 10) return;
        payloadLen = Number(buffer.readBigUInt64BE(2));
        offset = 10;
      }

      const maskSize = masked ? 4 : 0;
      const totalLen = offset + maskSize + payloadLen;
      if (buffer.length < totalLen) return;
      let payload = buffer.subarray(offset + maskSize, totalLen);
      if (masked) {
        const mask = buffer.subarray(offset, offset + 4);
        payload = Buffer.from(payload);
        for (let i = 0; i < payload.length; i += 1) {
          payload[i] ^= mask[i % 4];
        }
      }
      buffer = buffer.subarray(totalLen);
      if (opcode === 0x08) {
        socket.end();
        return;
      }
      if (opcode === 0x09) {
        sendWsFrame(socket, 0x0a, payload);
        continue;
      }
      if (opcode === 0x01) {
        handleSocketIOMessage(socket, payload.toString("utf-8"), mockSid);
      }
    }
  });
  socket.on("error", () => {});
  socket.on("close", () => {});
}

function startMockServer(port = DEFAULT_PORT) {
  return new Promise((resolve, reject) => {
    if (server) {
      resolve({ port: server.address()?.port ?? port, alreadyRunning: true });
      return;
    }
    server = http.createServer((req, res) => {
      handleRequest(req, res).catch((err) => {
        console.error("[MockServer] Unhandled error:", err);
        json(res, 500, { success: false, error: "Internal mock error" });
      });
    });
    server.on("connection", (socket) => {
      openSockets.add(socket);
      socket.on("close", () => openSockets.delete(socket));
    });
    server.on("upgrade", (req, socket) => handleWebSocketUpgrade(req, socket));
    server.on("error", reject);
    server.listen(port, "127.0.0.1", () => {
      console.log(`[MockServer] Listening on http://127.0.0.1:${port}`);
      resolve({ port });
    });
  });
}

function stopMockServer() {
  return new Promise((resolve) => {
    if (!server) {
      resolve();
      return;
    }
    for (const socket of openSockets) {
      socket.destroy();
    }
    openSockets.clear();
    server.close(() => {
      console.log("[MockServer] Stopped");
      server = null;
      resolve();
    });
  });
}

export {
  DEFAULT_PORT,
  clearRequestLog,
  getMockBehavior,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  setMockBehaviors,
  startMockServer,
  stopMockServer,
};
