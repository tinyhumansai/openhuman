import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function bootIntoOnboarding(page: Page, userId: string): Promise<void> {
  await bootRuntimeReadyGuestPage(page);
  await callCoreRpc('openhuman.auth_store_session', {
    token: `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${Buffer.from(
      JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
    ).toString('base64url')}.local`,
    userId,
    user: { _id: 'local', id: 'local', name: 'Local User', email: 'local@openhuman.local' },
  });
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/custom/inference');
  await page.reload();
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Onboarding custom configuration flow', () => {
  test('advanced path supports back navigation through final setup', async ({ page }) => {
    await bootIntoOnboarding(page, 'pw-onboarding-config');

    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible();
    await page.getByTestId('onboarding-next-button').click();

    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible();
    await page.getByRole('button', { name: /Back/ }).click();
    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible();
    await page.getByTestId('onboarding-next-button').click();

    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible({
      timeout: 20_000,
    });
    // Search is optional: skipping goes straight to the final step.
    await page.getByTestId('onboarding-search-skip').click();

    await expect(page.getByTestId('onboarding-custom-vault-step')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('onboarding-next-button')).toBeEnabled();
  });
});
