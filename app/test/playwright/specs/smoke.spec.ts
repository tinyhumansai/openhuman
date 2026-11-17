import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

test.describe('Smoke', () => {
  test('loads the browser-hosted app against the standalone core', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-smoke-user');

    await expect(page.locator('#root')).toBeVisible();
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/(home|chat)/);
    await expect(page.getByRole('navigation', { name: 'Home' })).toHaveCount(1);

    const bootstrap = await page.evaluate(() => ({
      rpcUrl: localStorage.getItem('openhuman_core_rpc_url'),
      rpcToken: localStorage.getItem('openhuman_core_rpc_token'),
    }));
    expect(bootstrap.rpcUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/rpc$/);
    expect(bootstrap.rpcToken?.length ?? 0).toBeGreaterThanOrEqual(16);
    await callCoreRpc('core.ping', {});

    const aboutApps = await callCoreRpc<unknown>('openhuman.about_app_list', {});
    const response = (aboutApps ?? {}) as Record<string, unknown>;
    const payload = 'result' in response ? response.result : response;
    expect(Array.isArray(payload)).toBe(true);
    expect((payload as unknown[]).length).toBeGreaterThan(0);
  });
});
