import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Crypto Payment Flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-crypto-payment-${slug}`);
    await page.goto('/#/settings/account');
  });

  test('account page shows the current Plan & billing handoff', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.getByText('Plan & billing', { exact: true })).toBeVisible();
    await expect(page.getByTestId('account-open-billing')).toBeVisible();
  });

  test('open billing dashboard button is present', async ({ page }) => {
    await waitForAppReady(page);
    await expect(page.getByTestId('account-open-billing')).toBeVisible();
  });

  test('keeps billing as an explicit account action', async ({ page }) => {
    await waitForAppReady(page);
    await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/settings/account');
    await expect(page.getByTestId('account-open-billing')).toBeVisible();
  });
});
