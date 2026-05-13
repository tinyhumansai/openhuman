import { json } from "../http.mjs";
import { createMockTunnel, getMockTunnels } from "../state.mjs";

export function handleWebhooks(ctx) {
  const { method, url, parsedBody, res } = ctx;
  const mockTunnels = getMockTunnels();

  if (method === "POST" && /^\/webhooks\/core\/?$/.test(url)) {
    const tunnel = createMockTunnel(parsedBody || {});
    mockTunnels.unshift(tunnel);
    json(res, 200, { success: true, data: tunnel });
    return true;
  }

  if (method === "GET" && /^\/webhooks\/core\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: mockTunnels });
    return true;
  }

  if (method === "GET" && /^\/webhooks\/core\/bandwidth\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: { remainingBudgetUsd: 10 } });
    return true;
  }

  const webhookCoreMatch = url.match(/^\/webhooks\/core\/([^/?]+)\/?(\?.*)?$/);
  if (webhookCoreMatch) {
    const [, tunnelId] = webhookCoreMatch;
    const tunnelIndex = mockTunnels.findIndex((entry) => entry.id === tunnelId);
    const tunnel = tunnelIndex >= 0 ? mockTunnels[tunnelIndex] : null;

    if (!tunnel) {
      json(res, 404, { success: false, error: "Tunnel not found" });
      return true;
    }

    if (method === "GET") {
      json(res, 200, { success: true, data: tunnel });
      return true;
    }

    if (method === "PATCH") {
      const updated = {
        ...tunnel,
        ...(parsedBody || {}),
        updatedAt: new Date().toISOString(),
      };
      mockTunnels[tunnelIndex] = updated;
      json(res, 200, { success: true, data: updated });
      return true;
    }

    if (method === "DELETE") {
      mockTunnels.splice(tunnelIndex, 1);
      json(res, 200, { success: true, data: tunnel });
      return true;
    }
  }

  // ── Webhook ingress (gap fill) ─────────────────────────────
  // The ingress side accepts inbound HTTP from third parties via a tunnel
  // ID; in e2e we just accept-and-acknowledge so the UI sees activity.
  if (method === "GET" && /^\/webhooks\/ingress\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [] });
    return true;
  }

  const ingressMatch = url.match(/^\/webhooks\/ingress\/([^/?]+)\/?(\?.*)?$/);
  if (ingressMatch) {
    if (method === "GET") {
      json(res, 200, {
        success: true,
        data: { id: ingressMatch[1], events: [] },
      });
      return true;
    }
    if (method === "POST") {
      json(res, 200, {
        success: true,
        data: {
          ingressId: ingressMatch[1],
          receivedAt: new Date().toISOString(),
        },
      });
      return true;
    }
  }

  return false;
}
