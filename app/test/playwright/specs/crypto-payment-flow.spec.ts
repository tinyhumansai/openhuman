import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Crypto Payment Flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-crypto-payment-${slug}`, '/settings/billing');
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

  test('shows the moved-to-web explanation on mount', async ({ page }) => {
    await waitForAppReady(page);
    // Billing no longer auto-opens the browser on mount; the panel explains that
    // billing moved to the web instead of showing an "opening browser" status.
    await expect(
      page.getByText(/Subscription changes, payment methods, credits, and invoices are now managed/)
    ).toBeVisible();
  });
});
