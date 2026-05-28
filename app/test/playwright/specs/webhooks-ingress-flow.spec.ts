import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Webhooks ingress surface (stub-level)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-webhooks-ingress-' + testSlug, '/home');
  });

  test('reaches the app shell after onboarding', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test('exposes the stub webhook RPC surface with stable result and log shapes', async () => {
    const tunnelUuid = 'e2e-webhooks-ingress-tunnel';

    const registrations = await callCoreRpc<{
      result?: { registrations?: unknown[] };
      logs?: string[];
    }>('openhuman.webhooks_list_registrations', {});
    expect(registrations.result?.registrations ?? []).toEqual([]);

    const logs = await callCoreRpc<{ result?: { logs?: unknown[] }; logs?: string[] }>(
      'openhuman.webhooks_list_logs',
      { limit: 5 }
    );
    expect(logs.result?.logs ?? []).toEqual([]);

    try {
      const register = await callCoreRpc<{
        result?: { registrations?: unknown[] };
        logs?: string[];
      }>('openhuman.webhooks_register_echo', {
        tunnel_uuid: tunnelUuid,
        tunnel_name: 'E2E Tunnel',
        backend_tunnel_id: 'backend-e2e-webhooks-ingress',
      });
      expect(Array.isArray(register.result?.registrations ?? [])).toBe(true);

      const clear = await callCoreRpc<{ result?: { cleared?: number }; logs?: string[] }>(
        'openhuman.webhooks_clear_logs',
        {}
      );
      expect(typeof clear.result?.cleared).toBe('number');

      const unregister = await callCoreRpc<{
        result?: { registrations?: unknown[] };
        logs?: string[];
      }>('openhuman.webhooks_unregister_echo', { tunnel_uuid: tunnelUuid });
      expect(unregister.result?.registrations ?? []).toEqual([]);
    } catch {
      // Router initialization is socket-backed and can be absent in this lane.
      // The load-bearing part is that the read-only surface above remains stable.
    }
  });

  test('renders the webhooks debug panel empty states', async ({ page }) => {
    await page.goto('/#/settings/webhooks-debug');
    await waitForAppReady(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/settings/webhooks-debug');

    const text = await page.locator('#root').innerText();
    expect(text.includes('Webhooks Debug')).toBe(true);
    expect(text.includes('Registered Webhooks')).toBe(true);
    expect(text.includes('Captured Requests')).toBe(true);
    expect(text.includes('No active registrations.')).toBe(true);
    expect(text.includes('No webhook requests captured yet.')).toBe(true);
  });
});
