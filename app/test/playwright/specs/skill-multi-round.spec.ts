import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Multi-round tool conversation smoke', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-skill-multi-round-' + testSlug, '/chat');
  });

  test('loads /chat after login for agent tool use', async ({ page }) => {
    await waitForAppReady(page);

    const hash = await page.evaluate(() => window.location.hash);
    expect(String(hash)).toContain('/chat');

    const text = await page.locator('#root').innerText();
    expect(
      ['Threads', 'New thread', 'Type a message', 'Chat'].some(marker => text.includes(marker))
    ).toBe(true);
  });
});
