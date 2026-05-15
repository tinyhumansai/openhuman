import { json } from "../http.mjs";

// Gap fill: cron-job and webhook-trigger configuration endpoints stored on
// the user's settings document. The real backend persists arrays; mock just
// returns empty lists and accepts writes as no-ops.
export function handleCron(ctx) {
  const { method, url, parsedBody, res } = ctx;

  if (method === "GET" && /^\/settings\/cron-jobs\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }
  if (method === "POST" && /^\/settings\/cron-jobs\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        id: "cron_mock_" + Date.now(),
        ...(parsedBody || {}),
        createdAt: new Date().toISOString(),
      },
    });
    return true;
  }
  const cronItem = url.match(/^\/settings\/cron-jobs\/([^/?]+)\/?(\?.*)?$/);
  if (cronItem && (method === "PATCH" || method === "DELETE")) {
    json(res, 200, {
      success: true,
      data: { id: cronItem[1], deleted: method === "DELETE" },
    });
    return true;
  }

  if (
    method === "GET" &&
    /^\/settings\/webhooks-triggers\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: [] });
    return true;
  }
  if (method === "POST" && /^\/settings\/webhooks-triggers\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        id: "trg_mock_" + Date.now(),
        ...(parsedBody || {}),
        createdAt: new Date().toISOString(),
      },
    });
    return true;
  }
  const trgItem = url.match(
    /^\/settings\/webhooks-triggers\/([^/?]+)\/?(\?.*)?$/,
  );
  if (trgItem && (method === "PATCH" || method === "DELETE")) {
    json(res, 200, {
      success: true,
      data: { id: trgItem[1], deleted: method === "DELETE" },
    });
    return true;
  }

  return false;
}
