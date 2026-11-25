import { expect, type Page, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function clickTestId(page: Page, testId: string): Promise<void> {
  const locator = page.getByTestId(testId);
  await expect(locator).toBeVisible();
  await expect(locator).toBeEnabled();
  await locator.click();
}

async function bootLocalOnboarding(page: Page, userId: string): Promise<void> {
  await resetMock().catch(() => undefined);
  await bootRuntimeReadyGuestPage(page);
  const payload = Buffer.from(
    JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  await callCoreRpc('openhuman.auth_store_session', {
    token: `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.local`,
    userId,
    user: { _id: 'local', id: 'local', name: 'Local User', email: 'local@openhuman.local' },
  });
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/custom/inference');
  await page.reload();
  await waitForAppReady(page);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/onboarding\/custom\/inference/);
}

test.describe('Onboarding modes', () => {
  test('TinyHumans sessions land directly in chat', async ({ page }) => {
    await resetMock().catch(() => undefined);
    await bootRuntimeReadyGuestPage(page);
    const userId = 'pw-onboarding-cloud';
    const payload = Buffer.from(
      JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
    ).toString('base64url');
    await callCoreRpc('openhuman.auth_store_session', {
      token: `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.sig`,
      userId,
      user: { _id: userId, id: userId, displayName: 'Playwright User' },
    });
    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
    await page.goto('/#/onboarding/welcome');
    await waitForAppReady(page);
    await expect
      .poll(() => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/chat/);
    const completed = await callCoreRpc<boolean | { result?: boolean }>(
      'openhuman.config_get_onboarding_completed',
      {}
    );
    expect(typeof completed === 'boolean' ? completed : completed?.result).toBe(true);
  });

  test('advanced custom path walks the three custom wizard steps and finishes on home', async ({
    page,
  }) => {
    await bootLocalOnboarding(page, 'pw-onboarding-custom');

    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible();
    await clickTestId(page, 'onboarding-next-button');

    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible();
    await clickTestId(page, 'onboarding-search-skip');
    await expect
      .poll(() => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/onboarding/custom/vault');

    await expect(page.getByTestId('onboarding-custom-vault-step')).toBeVisible();
    // Voice, OAuth and embeddings are no longer wizard steps.
    for (const retired of ['voice', 'oauth', 'embeddings']) {
      await expect(page.getByTestId(`onboarding-custom-${retired}-step`)).toHaveCount(0);
    }
    await clickTestId(page, 'onboarding-next-button');
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/(home|chat)/);

    await expect
      .poll(async () => {
        const completed = await callCoreRpc<boolean | { result?: boolean }>(
          'openhuman.config_get_onboarding_completed',
          {}
        );
        return typeof completed === 'boolean' ? completed : Boolean(completed?.result);
      })
      .toBe(true);
  });
});
