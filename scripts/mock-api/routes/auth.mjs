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

  // Login-token consume. The core POSTs to `/auth/login-token/consume` with the
  // token in a JSON body `{ token, audience? }` and parses `{ success, data: { jwt } }`
  // (see crates/openhuman-tinyhumans/src/session/client.rs `consume_login_token`). The legacy path-param route
  // `/telegram/login-tokens/:token/consume` was removed backend-side but is kept
  // here as a harmless alias in case an older client is exercised.
  if (
    method === "POST" &&
    (/^\/auth\/login-token\/consume\/?$/.test(url) ||
      /^\/telegram\/login-tokens\/[^/]+\/consume\/?$/.test(url))
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
    json(res, 200, { success: true, data: { jwt } });
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
    // A refusal on a live session (plan, WAF, proxy): only a 401 expires it.
    if (mockBehavior.session === "forbidden") {
      json(res, 403, { success: false, error: "Forbidden", errorCode: "FORBIDDEN" });
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
    // Sign-in with a loopback `redirectUri` (the browser dev build passes
    // `<vite>/__dev-auth`): answer like the real backend after a successful
    // provider login — redirect back with a session JWT and `key=auth`,
    // echoing `state` so the app's nonce check passes. The backend accepts
    // only http loopback redirect targets; so does the mock.
    const signInRedirect = loopbackSignInRedirect(url);
    if (signInRedirect) {
      setCors(res);
      res.writeHead(302, { Location: signInRedirect });
      res.end();
      return true;
    }
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

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

/**
 * The provider-login callback for an http loopback `redirectUri`, or `null`
 * when the request carries none (or a non-loopback one). Exported for tests.
 */
export function loopbackSignInRedirect(url) {
  const query = new URL(url, "http://mock.invalid").searchParams;
  const redirectUri = query.get("redirectUri");
  if (!redirectUri) return null;
  let target;
  try {
    target = new URL(redirectUri);
  } catch {
    return null;
  }
  if (target.protocol !== "http:" || !LOOPBACK_HOSTS.has(target.hostname)) {
    return null;
  }
  target.searchParams.set("token", MOCK_JWT);
  target.searchParams.set("key", "auth");
  const state = query.get("state");
  if (state) target.searchParams.set("state", state);
  return target.toString();
}
