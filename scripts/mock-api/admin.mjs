import { json } from "./http.mjs";
import {
  clearRequestLog,
  getMockBehavior,
  getRequestLog,
  resetMockBehavior,
  resetMockTunnels,
  setMockBehavior,
  setMockBehaviors,
} from "./state.mjs";

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
  return false;
}
