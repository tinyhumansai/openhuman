import { json, setCors } from "../http.mjs";
import {
  behavior,
  getDelayMs,
  getMockUser,
  MOCK_JWT,
  sleep,
} from "../state.mjs";

export async function handleAuth(ctx) {
  const { method, url, res, origin } = ctx;
  const mockBehavior = behavior();

  if (
    method === "POST" &&
    /^\/telegram\/login-tokens\/[^/]+\/consume\/?$/.test(url)
  ) {
    if (mockBehavior.token === "expired") {
      json(res, 401, { success: false, error: "Token expired or invalid" });
      return true;
    }
    if (mockBehavior.token === "invalid") {
      json(res, 401, { success: false, error: "Invalid token" });
      return true;
    }
    const jwt = mockBehavior.jwt ? `${MOCK_JWT}-${mockBehavior.jwt}` : MOCK_JWT;
    json(res, 200, { success: true, data: { jwtToken: jwt } });
    return true;
  }

  if (method === "POST" && /^\/auth\/desktop-exchange\/?$/.test(url)) {
    json(res, 200, {
      sessionToken: "mock-session-token",
      user: { id: "user-123", firstName: "Test", username: "testuser" },
    });
    return true;
  }

  if (
    method === "GET" &&
    (/^\/telegram\/me\/?(\?.*)?$/.test(url) ||
      /^\/auth\/me\/?(\?.*)?$/.test(url))
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
      return true;
    }
    if (mockBehavior.session === "revoked") {
      json(res, 401, { success: false, error: "Unauthorized" });
      return true;
    }
    json(res, 200, { success: true, data: getMockUser() });
    return true;
  }

  if (method === "GET" && /^\/auth\/integrations\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }

  if (method === "POST" && /^\/auth\/email\/send-link\/?$/.test(url)) {
    // Gap fill: passwordless magic-link send. Always succeed in mock.
    json(res, 200, {
      success: true,
      data: { sent: true, expiresAt: new Date(Date.now() + 600000).toISOString() },
    });
    return true;
  }

  if (method === "GET" && /^\/auth\/[^/]+\/login\/?(\?.*)?$/.test(url)) {
    const redirectUrl = `${origin}/mock-oauth`;
    if (url.includes("responseType=json")) {
      json(res, 200, { success: true, data: { oauthUrl: redirectUrl } });
      return true;
    }
    setCors(res);
    res.writeHead(302, { Location: redirectUrl });
    res.end();
    return true;
  }

  if (method === "GET" && /^\/auth\/telegram\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.telegramDuplicate === "true") {
      json(res, 409, {
        success: false,
        error: "Telegram account already linked to another user",
      });
      return true;
    }
    json(res, 200, {
      success: true,
      data: { oauthUrl: `${origin}/mock-telegram-oauth` },
    });
    return true;
  }

  if (method === "GET" && /^\/auth\/notion\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.notionTokenRevoked === "true") {
      json(res, 401, { success: false, error: "OAuth token has been revoked" });
      return true;
    }
    const workspace = mockBehavior.notionWorkspace || "Test User's Workspace";
    json(res, 200, {
      success: true,
      data: { oauthUrl: `${origin}/mock-notion-oauth`, workspace },
    });
    return true;
  }

  if (method === "GET" && /^\/auth\/google\/connect\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.gmailTokenRevoked === "true") {
      json(res, 401, { success: false, error: "OAuth token has been revoked" });
      return true;
    }
    if (mockBehavior.gmailTokenExpired === "true") {
      json(res, 401, { success: false, error: "OAuth token has expired" });
      return true;
    }
    json(res, 200, {
      success: true,
      data: { oauthUrl: `${origin}/mock-google-oauth` },
    });
    return true;
  }

  if (method === "POST" && /^\/auth\/telegram\/?$/.test(url)) {
    // Gap fill: telegram login callback exchange.
    json(res, 200, { success: true, data: { jwtToken: MOCK_JWT } });
    return true;
  }

  // /mock-oauth, /mock-oauth/<provider>, and the legacy
  // /mock-<provider>-oauth aliases are handled by routes/oauth.mjs, which
  // actually completes the OAuth flow via deep links.

  return false;
}
