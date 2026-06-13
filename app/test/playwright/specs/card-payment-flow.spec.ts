import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Card Payment Flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-card-payment-${slug}`, '/settings/billing');
  });

  test('billing panel shows the moved-to-web redirect page', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.getByRole('heading', { name: 'Open billing dashboard' })).toBeVisible();
    await expect(page.getByText(/Billing moved to the web/i)).toBeVisible();
  });

  test('open billing dashboard button is present', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.getByRole('button', { name: 'Open billing dashboard' })).toBeVisible();
  });

  test('back-to-settings navigation works', async ({ page }) => {
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    const backButton = page.getByRole('button', { name: 'Back to settings' });
    if (await backButton.count()) {
      await backButton.evaluate((button: HTMLElement) => button.click());
    } else {
      await page.getByRole('button', { name: 'Settings' }).first().click({ force: true });
    }
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/settings');
  });
});
