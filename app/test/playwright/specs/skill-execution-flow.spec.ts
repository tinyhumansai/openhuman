import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Skill discovery (UI + core RPC)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-skill-execution-' + testSlug, '/home');
  });

  test('lands the user on a logged-in shell', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test('core.ping responds over the same JSON-RPC URL the UI uses', async () => {
    const ping = await callCoreRpc<{ ok?: boolean }>('core.ping', {});
    expect(ping.ok).toBe(true);
  });

  test('skills UI surface shows installed tools', async ({ page }) => {
    // /skills redirects to /connections (Phase 2 rename)
    await page.goto('/#/connections');
    await waitForAppReady(page);

    const hash = await page.evaluate(() => window.location.hash);
    expect(String(hash)).toContain('/connections');

    const text = await page.locator('#root').innerText();
    expect(
      ['Composio Integrations', 'Apps', 'Messaging', 'Gmail', 'Notion', 'GitHub'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });
});
