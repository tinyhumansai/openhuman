import { html, json, setCors } from "../http.mjs";
import { behavior, MOCK_JWT } from "../state.mjs";

/**
 * Mock OAuth provider landing page + callback handlers.
 *
 * The real flow looks like:
 *   1. App calls `GET /auth/<provider>/connect` → backend returns `oauthUrl`
 *      pointing at the actual provider (Google, Notion, Slack, …).
 *   2. App opens that URL in the system browser.
 *   3. User logs in at the provider, provider redirects back to the backend
 *      callback, the backend exchanges the code and finally redirects to the
 *      desktop deep link `openhuman://oauth/success?integrationId=…&provider=…`
 *      (see app/src/utils/desktopDeepLinkListener.ts).
 *   4. The Tauri shell receives the deep link, the app marks the integration
 *      connected.
 *
 * For e2e we collapse all of that into local HTTP. `/auth/<provider>/connect`
 * already points the app at `${origin}/mock-<provider>-oauth`; this module
 * makes that page actually finish the dance by issuing the deep link.
 *
 * Behavior knobs (via `setMockBehavior`):
 *   oauthAutoRedirectMs       — delay before the deep-link redirect fires
 *                                (default 50). Set `manual` to require a click.
 *   oauthIntegrationId        — integrationId param in the success deep link
 *                                (default `mock-<provider>-integration`).
 *   oauthForceError           — when "true", redirect to the error deep link.
 *   oauthErrorCode            — error code passed to the error deep link.
 *
 * Query-string overrides (per request, win over behavior knobs):
 *   ?provider=<name>          — overrides provider inferred from the path.
 *   ?integrationId=<id>       — overrides the integrationId emitted.
 *   ?manual=1                 — disable auto-redirect (test wants to click).
 *   ?error=<code>             — emit an error deep link instead of success.
 */
export function handleOAuth(ctx) {
  const { method, url, parsedBody, res } = ctx;

  // /mock-oauth/<provider> and the legacy /mock-<provider>-oauth aliases.
  const newStyle = url.match(/^\/mock-oauth\/([a-z][a-z0-9_-]*)\/?(\?.*)?$/i);
  const legacy = url.match(
    /^\/mock-(telegram|notion|google|gmail|slack|discord|twitter|github)-oauth\/?(\?.*)?$/i,
  );
  const legacyGeneric = !newStyle && !legacy && /^\/mock-oauth\/?(\?.*)?$/.test(url);

  if (method === "GET" && (newStyle || legacy || legacyGeneric)) {
    const provider = newStyle?.[1] || legacy?.[1] || "generic";
    const params = parseQuery(url);
    const mockBehavior = behavior();
    const integrationId =
      params.integrationId ||
      mockBehavior.oauthIntegrationId ||
      `mock-${provider}-integration`;
    const errorCode =
      params.error ||
      (mockBehavior.oauthForceError === "true"
        ? mockBehavior.oauthErrorCode || "access_denied"
        : null);
    const manual =
      params.manual === "1" || mockBehavior.oauthAutoRedirectMs === "manual";
    const autoRedirectMs = manual
      ? null
      : clampDelay(mockBehavior.oauthAutoRedirectMs, 50);

    const target = errorCode
      ? `openhuman://oauth/error?provider=${encodeURIComponent(
          params.provider || provider,
        )}&error=${encodeURIComponent(errorCode)}`
      : `openhuman://oauth/success?integrationId=${encodeURIComponent(
          integrationId,
        )}&provider=${encodeURIComponent(params.provider || provider)}`;

    html(res, 200, renderOAuthPage({ provider, target, autoRedirectMs, errorCode }));
    return true;
  }

  // Generic callback exchange. Real providers each hit their own
  // backend-specific URL; for e2e a single endpoint per provider that
  // always returns a session token is enough.
  const callbackMatch = url.match(/^\/auth\/([a-z][a-z0-9_-]*)\/callback\/?(\?.*)?$/i);
  if (method === "GET" && callbackMatch) {
    const provider = callbackMatch[1];
    const params = parseQuery(url);
    const mockBehavior = behavior();

    if (mockBehavior.oauthForceError === "true" || params.error) {
      const errorCode =
        params.error || mockBehavior.oauthErrorCode || "access_denied";
      // Redirect to the desktop error deep link.
      setCors(res);
      res.writeHead(302, {
        Location: `openhuman://oauth/error?provider=${encodeURIComponent(
          provider,
        )}&error=${encodeURIComponent(errorCode)}`,
      });
      res.end();
      return true;
    }

    const integrationId =
      params.integrationId ||
      mockBehavior.oauthIntegrationId ||
      `mock-${provider}-integration`;
    setCors(res);
    res.writeHead(302, {
      Location: `openhuman://oauth/success?integrationId=${encodeURIComponent(
        integrationId,
      )}&provider=${encodeURIComponent(provider)}`,
    });
    res.end();
    return true;
  }

  // Backend-style code-for-token POST exchange, in case any provider
  // routes through the desktop app rather than the deep link.
  if (
    method === "POST" &&
    /^\/auth\/[a-z][a-z0-9_-]*\/exchange\/?$/i.test(url)
  ) {
    const provider = url.split("/")[2] ?? "generic";
    const mockBehavior = behavior();
    if (mockBehavior.oauthForceError === "true") {
      json(res, 400, {
        success: false,
        error: mockBehavior.oauthErrorCode || "access_denied",
      });
      return true;
    }
    const integrationId =
      parsedBody?.integrationId ||
      mockBehavior.oauthIntegrationId ||
      `mock-${provider}-integration`;
    json(res, 200, {
      success: true,
      data: {
        provider,
        integrationId,
        sessionToken: "mock-session-token",
        jwtToken: MOCK_JWT,
      },
    });
    return true;
  }

  return false;
}

function parseQuery(url) {
  const qIndex = url.indexOf("?");
  if (qIndex < 0) return {};
  const out = {};
  const search = url.slice(qIndex + 1);
  for (const pair of search.split("&")) {
    if (!pair) continue;
    const eq = pair.indexOf("=");
    const key = eq < 0 ? pair : pair.slice(0, eq);
    const raw = eq < 0 ? "" : pair.slice(eq + 1);
    try {
      out[decodeURIComponent(key)] = decodeURIComponent(raw.replace(/\+/g, " "));
    } catch {
      out[key] = raw;
    }
  }
  return out;
}

function clampDelay(raw, fallback) {
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0) return fallback;
  return Math.min(parsed, 30000);
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[c]);
}

function renderOAuthPage({ provider, target, autoRedirectMs, errorCode }) {
  const safeTarget = escapeHtml(target);
  const safeProvider = escapeHtml(provider);
  const heading = errorCode
    ? `${safeProvider} — sign-in failed (${escapeHtml(errorCode)})`
    : `${safeProvider} — mock sign-in`;
  const blurb = errorCode
    ? "This mock provider is simulating a failed OAuth. The desktop app should receive an error deep link."
    : "This is the mock OAuth provider. The desktop app should receive a success deep link.";

  const autoRedirectScript =
    autoRedirectMs === null
      ? ""
      : `<script>setTimeout(function(){window.location.href=${JSON.stringify(
          target,
        )};}, ${Number(autoRedirectMs)});</script>`;

  const metaRefresh =
    autoRedirectMs === null
      ? ""
      : `<meta http-equiv="refresh" content="${(Number(autoRedirectMs) /
          1000).toFixed(2)};url=${safeTarget}" />`;

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Mock OAuth · ${safeProvider}</title>
  ${metaRefresh}
  <style>
    body { font: 16px/1.4 system-ui, sans-serif; max-width: 480px; margin: 80px auto; padding: 0 24px; color: #1d1d1d; }
    h1 { font-size: 20px; margin: 0 0 12px; }
    p  { margin: 0 0 16px; color: #555; }
    a.button { display: inline-block; padding: 10px 16px; background: #4A83DD; color: #fff; border-radius: 8px; text-decoration: none; font-weight: 600; }
    code { background: #f3f3f3; padding: 2px 6px; border-radius: 4px; font-size: 13px; }
  </style>
</head>
<body>
  <h1>${heading}</h1>
  <p>${blurb}</p>
  <p>Target: <code>${safeTarget}</code></p>
  <p><a class="button" id="continue" href="${safeTarget}">Continue to OpenHuman</a></p>
  ${autoRedirectScript}
</body>
</html>`;
}
