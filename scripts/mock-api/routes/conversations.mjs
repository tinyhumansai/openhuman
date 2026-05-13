import { json } from "../http.mjs";

// Gap fill: conversations / messages / channels / notifications.
//
// The real backend serves rich, paginated data here; for e2e we return empty
// lists wrapped in the standard envelope. Specs that need richer fixtures can
// override via `setMockBehavior` and a future scenario knob.
export function handleConversations(ctx) {
  const { method, url, parsedBody, res } = ctx;

  // /conversations
  if (method === "GET" && /^\/conversations\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }
  if (method === "POST" && /^\/conversations\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        id: "conv_mock_" + Date.now(),
        ...(parsedBody || {}),
        createdAt: new Date().toISOString(),
      },
    });
    return true;
  }
  const conversationItemMatch = url.match(
    /^\/conversations\/([^/?]+)\/?(\?.*)?$/,
  );
  if (conversationItemMatch) {
    if (method === "GET") {
      json(res, 200, {
        success: true,
        data: { id: conversationItemMatch[1], messages: [] },
      });
      return true;
    }
    if (method === "DELETE") {
      json(res, 200, { success: true, data: { deleted: true } });
      return true;
    }
  }

  // /messages
  if (method === "GET" && /^\/messages\/matches\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: { matches: [] } });
    return true;
  }
  if (method === "GET" && /^\/messages\/paging\/pages\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { pages: [], nextCursor: null },
    });
    return true;
  }
  if (method === "GET" && /^\/messages\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }
  if (method === "POST" && /^\/messages\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        id: "msg_mock_" + Date.now(),
        ...(parsedBody || {}),
        createdAt: new Date().toISOString(),
      },
    });
    return true;
  }

  // /channels
  if (method === "GET" && /^\/channels\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }
  const channelItemMatch = url.match(/^\/channels\/([^/?]+)\/?(\?.*)?$/);
  if (channelItemMatch) {
    if (method === "GET") {
      json(res, 200, {
        success: true,
        data: { id: channelItemMatch[1], name: "Mock Channel" },
      });
      return true;
    }
    if (method === "PATCH") {
      json(res, 200, {
        success: true,
        data: { id: channelItemMatch[1], ...(parsedBody || {}) },
      });
      return true;
    }
  }

  // /notifications
  if (method === "GET" && /^\/notifications\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }

  return false;
}
