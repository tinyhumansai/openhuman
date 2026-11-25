import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Webhooks ingress surface (stub-level)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-webhooks-ingress-' + testSlug, '/home');
  });

  test('reaches the app shell after onboarding', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
  });

  // No Rust owner test exercises this RPC wire surface against its router;
  // retain these assertions until that integration coverage exists.
  test('exposes the stub webhook RPC surface with stable result and log shapes', async () => {
    const tunnelUuid = 'e2e-webhooks-ingress-tunnel';

    const registrations = await callCoreRpc<{
      result: { registrations: unknown[] };
      logs: string[];
    }>('openhuman.webhooks_list_registrations', {});
    expect(registrations.result.registrations).toEqual([]);
    expect(registrations.logs[0]).toContain('webhooks.list_registrations returned 0');

    const logs = await callCoreRpc<{ result: { logs: unknown[] }; logs: string[] }>(
      'openhuman.webhooks_list_logs',
      { limit: 5 }
    );
    expect(logs.result.logs).toEqual([]);
    expect(logs.logs[0]).toContain('webhooks.list_logs returned 0');

    let register: { result: { registrations: unknown[] }; logs: string[] };
    try {
      register = await callCoreRpc('openhuman.webhooks_register_echo', {
        tunnel_uuid: tunnelUuid,
        tunnel_name: 'E2E Tunnel',
        backend_tunnel_id: 'backend-e2e-webhooks-ingress',
      });
    } catch (error) {
      if (!(error instanceof Error) || !error.message.includes('webhook router not initialized')) {
        throw error;
      }
      return;
    }

    expect(register.result.registrations.length).toBeGreaterThanOrEqual(1);
    expect(register.logs[0]).toContain(`webhooks.register_echo registered tunnel ${tunnelUuid}`);

    const clear = await callCoreRpc<{ result: { cleared: number }; logs: string[] }>(
      'openhuman.webhooks_clear_logs',
      {}
    );
    expect(clear.result.cleared).toBe(0);
    expect(clear.logs[0]).toContain('webhooks.clear_logs removed 0');

    const unregister = await callCoreRpc<{ result: { registrations: unknown[] }; logs: string[] }>(
      'openhuman.webhooks_unregister_echo',
      { tunnel_uuid: tunnelUuid }
    );
    expect(unregister.result.registrations).toEqual([]);
    expect(unregister.logs[0]).toContain(`webhooks.unregister_echo removed tunnel ${tunnelUuid}`);
  });
});
