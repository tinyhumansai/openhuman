import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Linux CEF deb package runtime', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-linux-cef-runtime-${slug}`, '/home');
  });

  test('core RPC endpoint responds to ping', async () => {
    const result = await callCoreRpc<{ ok?: boolean }>('core.ping', {});
    expect(result.ok).toBe(true);
  });

  test('core version is accessible via JSON-RPC', async () => {
    const result = await callCoreRpc<unknown>('core.version', {});
    expect(typeof result).not.toBe('undefined');
  });

  test('main web shell is created and visible', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected', 'Home', 'Chat'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test.skip('native core_rpc_url / tray / CEF packaging assertions are desktop-only', async () => {});
});
