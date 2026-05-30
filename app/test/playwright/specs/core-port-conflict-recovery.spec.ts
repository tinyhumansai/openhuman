import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Core port conflict recovery', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-core-port-conflict-' + testSlug, '/home');
  });

  test('startup-integrity check reaches a usable screen', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected', 'Welcome', 'Get Started'].some(
        marker => text.includes(marker)
      )
    ).toBe(true);
  });

  test.skip('second instance surfaces clear conflict dialog once a visible banner exists', async () => {});
});
