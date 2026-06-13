import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage } from '../helpers/core-rpc';

test.describe('Smoke', () => {
  test('loads the browser-hosted app against the standalone core', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-smoke-user');

    await expect(page.locator('#root')).toBeVisible();
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/(home|chat)/);
    await expect(page.locator('[data-testid="bottom-tab-bar"], nav')).toHaveCount(1);
  });
});
