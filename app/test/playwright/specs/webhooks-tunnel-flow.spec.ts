import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

type RequestLogEntry = { method?: string; url?: string; body?: string };

async function mockFetch(path: string, init?: RequestInit) {
  const response = await fetch(MOCK_BASE + path, init);
  if (!response.ok) {
    throw new Error('mock request failed: ' + response.status + ' ' + path);
  }
  return response.json() as Promise<{ data?: unknown }>;
}

async function resetMock() {
  await mockFetch('/__admin/reset', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keepBehavior: false, keepRequests: false }),
  });
}

async function getRequestLog(): Promise<RequestLogEntry[]> {
  const payload = await mockFetch('/__admin/requests');
  return (payload.data as RequestLogEntry[]) ?? [];
}

function unwrapRpcValue<T = unknown>(raw: unknown): T | undefined {
  if (raw === null || raw === undefined) return undefined;
  if (typeof raw === 'object' && raw !== null && 'result' in (raw as Record<string, unknown>)) {
    const inner = (raw as { result?: unknown }).result;
    if (inner !== undefined) return inner as T;
  }
  return raw as T;
}

async function waitForRequest(
  method: string,
  urlFragment: string,
  timeoutMs = 10_000
): Promise<RequestLogEntry | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const log = await getRequestLog();
    const match = log.find(entry => entry.method === method && entry.url?.includes(urlFragment));
    if (match) return match;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  return undefined;
}

test.describe('Webhook tunnel CRUD (UI + core RPC + mock backend)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await resetMock();
    await bootAuthenticatedPage(page, 'pw-webhooks-tunnel-' + testSlug, '/home');
  });

  test('reached the logged-in shell after onboarding', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test('creates a tunnel, lists it, deletes it, and matches mock-backend traffic', async () => {
    const tunnelName = `e2e-tunnel-${Date.now()}`;
    const created = await callCoreRpc<unknown>('openhuman.webhooks_create_tunnel', {
      name: tunnelName,
      description: 'Created by webhooks-tunnel-flow Playwright spec.',
    });
    const createdTunnel = unwrapRpcValue<{ id?: string; uuid?: string; name?: string }>(created);
    const tunnelId = createdTunnel?.id;
    expect(typeof tunnelId).toBe('string');
    expect(createdTunnel?.name).toBe(tunnelName);
    expect(await waitForRequest('POST', '/webhooks/core', 10_000)).toBeDefined();

    const listed = await callCoreRpc<unknown>('openhuman.webhooks_list_tunnels', {});
    const tunnels = unwrapRpcValue<Array<{ id?: string; name?: string }>>(listed) ?? [];
    const found = tunnels.find(tunnel => tunnel?.id === tunnelId);
    expect(found?.name).toBe(tunnelName);
    expect(await waitForRequest('GET', '/webhooks/core', 10_000)).toBeDefined();

    await callCoreRpc<unknown>('openhuman.webhooks_delete_tunnel', { id: tunnelId });
    expect(
      await waitForRequest(
        'DELETE',
        `/webhooks/core/${encodeURIComponent(String(tunnelId))}`,
        10_000
      )
    ).toBeDefined();

    const relisted = await callCoreRpc<unknown>('openhuman.webhooks_list_tunnels', {});
    const relistedTunnels = unwrapRpcValue<Array<{ id?: string }>>(relisted) ?? [];
    expect(relistedTunnels.some(tunnel => tunnel?.id === tunnelId)).toBe(false);
  });

  test('webhooks page loads (ComposeIO trigger history surface)', async ({ page }) => {
    await page.goto('/#/settings/webhooks-triggers');
    await waitForAppReady(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/settings/webhooks-triggers');

    const text = await page.locator('#root').innerText();
    expect(
      ['ComposeIO Triggers', 'ComposeIO', 'Archive', 'Refresh'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });
});
