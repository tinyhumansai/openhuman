import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Connectivity state differentiation (issue #1527)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-connectivity-diff-' + testSlug, '/home');
  });

  test('baseline app shell is ready in the browser lane', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.locator('#root')).toBeVisible();
  });
});
