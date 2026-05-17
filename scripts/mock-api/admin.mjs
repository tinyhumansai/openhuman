import { json } from "./http.mjs";
import { resetConversationFixturesState } from "./routes/conversations.mjs";
import { resetCronFixturesState } from "./routes/cron.mjs";
import {
  clearSocketEventLog,
  clearRequestLog,
  getSocketEventLog,
  listSocketSessions,
  getMockBehavior,
  getMockConversations,
  getMockCronJobs,
  getMockMessages,
  listMockLlmThreads,
  getMockWebhookTriggers,
  getRequestLog,
  resetMockBehavior,
  resetMockConversations,
  resetMockCronJobs,
  resetMockMessages,
  resetMockLlmThreads,
  resetSocketSessions,
  resetMockTunnels,
  resetMockWebhookTriggers,
  setMockBehavior,
  setMockBehaviors,
} from "./state.mjs";
import { disconnectMockSockets, emitMockSocketEvent } from "./socket.mjs";

export function handleAdmin(ctx) {
  const { method, url, parsedBody, res, getPort } = ctx;

  if (method === "GET" && /^\/__admin\/health\/?$/.test(url)) {
    json(res, 200, { ok: true, port: getPort() });
    return true;
  }
  if (method === "GET" && /^\/__admin\/requests\/?$/.test(url)) {
    json(res, 200, { success: true, data: getRequestLog() });
    return true;
  }
  if (method === "GET" && /^\/__admin\/behavior\/?$/.test(url)) {
    json(res, 200, { success: true, data: getMockBehavior() });
    return true;
  }
  if (method === "GET" && /^\/__admin\/state\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        requestCount: getRequestLog().length,
        conversations: getMockConversations(),
        messages: getMockMessages(),
        cronJobs: getMockCronJobs(),
        webhookTriggers: getMockWebhookTriggers(),
        llmThreads: listMockLlmThreads(),
        socketSessions: listSocketSessions(),
        socketEventCount: getSocketEventLog().length,
      },
    });
    return true;
  }
  if (method === "GET" && /^\/__admin\/socket\/sessions\/?$/.test(url)) {
    json(res, 200, { success: true, data: listSocketSessions() });
    return true;
  }
  if (method === "GET" && /^\/__admin\/socket\/events\/?$/.test(url)) {
    json(res, 200, { success: true, data: getSocketEventLog() });
    return true;
  }
  if (method === "POST" && /^\/__admin\/reset\/?$/.test(url)) {
    const keepBehavior = parsedBody?.keepBehavior === true;
    const keepRequests = parsedBody?.keepRequests === true;
    if (!keepBehavior) resetMockBehavior();
    if (!keepRequests) clearRequestLog();
    resetMockTunnels();
    resetMockConversations();
    resetMockMessages();
    resetMockCronJobs();
    resetMockWebhookTriggers();
    resetMockLlmThreads();
    resetConversationFixturesState();
    resetCronFixturesState();
    resetSocketSessions();
    json(res, 200, {
      success: true,
      data: {
        behavior: getMockBehavior(),
        requestCount: getRequestLog().length,
      },
    });
    return true;
  }
  if (method === "POST" && /^\/__admin\/behavior\/?$/.test(url)) {
    if (parsedBody?.behavior && typeof parsedBody.behavior === "object") {
      setMockBehaviors(parsedBody.behavior, parsedBody.mode);
    } else if (parsedBody?.key) {
      setMockBehavior(parsedBody.key, parsedBody.value ?? "");
    }
    json(res, 200, { success: true, data: getMockBehavior() });
    return true;
  }
  if (method === "POST" && /^\/__admin\/socket\/emit\/?$/.test(url)) {
    const delivered = emitMockSocketEvent({
      event: parsedBody?.event,
      data: parsedBody?.data,
      targetSid: parsedBody?.targetSid,
      targetUserId: parsedBody?.targetUserId,
      excludeSid: parsedBody?.excludeSid,
      delayMs: parsedBody?.delayMs,
    });
    json(res, 200, { success: true, data: { delivered } });
    return true;
  }
  if (method === "POST" && /^\/__admin\/socket\/disconnect\/?$/.test(url)) {
    const disconnected = disconnectMockSockets({
      targetSid: parsedBody?.targetSid,
      targetUserId: parsedBody?.targetUserId,
    });
    json(res, 200, { success: true, data: { disconnected } });
    return true;
  }
  if (method === "POST" && /^\/__admin\/socket\/clear-events\/?$/.test(url)) {
    clearSocketEventLog();
    json(res, 200, { success: true, data: [] });
    return true;
  }
  return false;
}
