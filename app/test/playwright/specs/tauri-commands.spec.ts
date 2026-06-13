import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Tauri commands', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-tauri-commands-${slug}`, '/home');
  });

  test('app chrome is visible', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected', 'Home', 'Chat'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test('browser lane exposes the core RPC URL and token bootstrap values', async ({ page }) => {
    const values = await page.evaluate(() => ({
      rpcUrl: window.localStorage.getItem('openhuman_core_rpc_url'),
      rpcToken: window.localStorage.getItem('openhuman_core_rpc_token'),
    }));
    expect(String(values.rpcUrl)).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/rpc$/);
    expect((values.rpcToken ?? '').length).toBeGreaterThanOrEqual(16);
  });

  test('core.ping succeeds through the same core RPC helper the web lane uses', async () => {
    const ping = await callCoreRpc<{ ok?: boolean }>('core.ping', {});
    expect(ping.ok).toBe(true);
  });

  test('openhuman.about_app_list round-trips over core RPC', async () => {
    const res = await callCoreRpc<unknown>('openhuman.about_app_list', {});
    const root = (res ?? {}) as Record<string, unknown>;
    const payload = root && typeof root === 'object' && 'result' in root ? root.result : root;
    expect(Array.isArray(payload)).toBe(true);
    expect((payload as unknown[]).length).toBeGreaterThan(0);
  });

  test.skip('native window.__TAURI_INTERNALS__.invoke checks are desktop-only and not available in the web lane', async () => {});
});
