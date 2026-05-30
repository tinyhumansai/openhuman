import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Skill OAuth UI smoke', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-skill-oauth-' + testSlug, '/skills');
  });

  test('skills page shows skill rows with actions after login', async ({ page }) => {
    await waitForAppReady(page);

    const hash = await page.evaluate(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const text = await page.locator('#root').innerText();
    expect(
      ['Composio Integrations', 'Connect', 'Setup', 'Manage', 'Channels'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });
});
