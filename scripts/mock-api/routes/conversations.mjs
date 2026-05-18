import { json } from "../http.mjs";
import {
  behavior,
  createMockId,
  fuzzyNumber,
  fuzzyPick,
  fuzzyTimestamp,
  getMockConversations,
  getMockMessages,
} from "../state.mjs";

let fixturesSeeded = false;

export function resetConversationFixturesState() {
  fixturesSeeded = false;
}

function ensureConversationFixtures() {
  if (fixturesSeeded) return;
  const conversations = getMockConversations();
  const messages = getMockMessages();
  if (conversations.length > 0 || messages.length > 0) {
    fixturesSeeded = true;
    return;
  }

  const mockBehavior = behavior();
  const count = Math.max(
    0,
    Number(mockBehavior.conversationCount || fuzzyNumber("conv:count", 1, 4)),
  );

  const titles = [
    "Daily standup",
    "Onboarding thread",
    "Billing follow-up",
    "Release prep",
    "Research scratchpad",
  ];

  for (let i = 0; i < count; i += 1) {
    const id = createMockId("conv");
    conversations.push({
      id,
      title: fuzzyPick(`conv:title:${i}`, titles, "Mock Conversation"),
      channel: fuzzyPick(
        `conv:channel:${i}`,
        ["web", "telegram", "discord"],
        "web",
      ),
      unreadCount: fuzzyNumber(`conv:unread:${i}`, 0, 3),
      archived: false,
      createdAt: fuzzyTimestamp(`conv:created:${i}`),
      updatedAt: fuzzyTimestamp(`conv:updated:${i}`, 3 * 24 * 60 * 60 * 1000),
    });

    const messageCount = fuzzyNumber(`conv:messages:${i}`, 1, 3);
    for (let j = 0; j < messageCount; j += 1) {
      messages.push({
        id: createMockId("msg"),
        conversationId: id,
        role: j % 2 === 0 ? "user" : "assistant",
        text:
          j % 2 === 0
            ? `Mock prompt ${j + 1} for ${id}`
            : `Mock response ${j + 1} for ${id}`,
        createdAt: fuzzyTimestamp(`msg:${i}:${j}`, 2 * 24 * 60 * 60 * 1000),
      });
    }
  }

  fixturesSeeded = true;
}

export function handleConversations(ctx) {
  const { method, url, parsedBody, res } = ctx;
  ensureConversationFixtures();
  const conversations = getMockConversations();
  const messages = getMockMessages();

  // /conversations
  if (method === "GET" && /^\/conversations\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: conversations });
    return true;
  }
  if (method === "POST" && /^\/conversations\/?$/.test(url)) {
    const created = {
      ...(parsedBody || {}),
      id: createMockId("conv"),
      title:
        typeof parsedBody?.title === "string" && parsedBody.title.trim()
          ? parsedBody.title.trim()
          : "Untitled mock conversation",
      channel: parsedBody?.channel || "web",
      unreadCount: 0,
      archived: false,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    };
    conversations.unshift(created);
    json(res, 200, {
      success: true,
      data: created,
    });
    return true;
  }
  const conversationItemMatch = url.match(
    /^\/conversations\/([^/?]+)\/?(\?.*)?$/,
  );
  if (conversationItemMatch) {
    const conversation = conversations.find(
      (entry) => entry.id === conversationItemMatch[1],
    );
    if (method === "GET") {
      json(res, 200, {
        success: true,
        data: {
          ...(conversation || {
            id: conversationItemMatch[1],
            title: "Mock Conversation",
            channel: "web",
          }),
          messages: messages.filter(
            (entry) => entry.conversationId === conversationItemMatch[1],
          ),
        },
      });
      return true;
    }
    if (method === "DELETE") {
      const index = conversations.findIndex(
        (entry) => entry.id === conversationItemMatch[1],
      );
      if (index >= 0) {
        conversations.splice(index, 1);
      }
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
    json(res, 200, { success: true, data: messages });
    return true;
  }
  if (method === "POST" && /^\/messages\/?$/.test(url)) {
    const created = {
      ...(parsedBody || {}),
      id: createMockId("msg"),
      conversationId: parsedBody?.conversationId || null,
      role: parsedBody?.role || "user",
      text:
        parsedBody?.text ||
        parsedBody?.content ||
        "Synthetic mock message created by the test harness",
      createdAt: new Date().toISOString(),
    };
    messages.push(created);
    json(res, 200, {
      success: true,
      data: created,
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
