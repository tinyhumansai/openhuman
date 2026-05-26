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

  test('opening-browser status copy is shown on mount', async ({ page }) => {
    await waitForAppReady(page);
    await expect(
      page
        .getByText(
          /Opening your browser|If your browser did not open, use the button above\.|The browser could not be opened automatically\./
        )
        .first()
    ).toBeVisible();
  });
});
