import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function clickOnboardingNext(page: Page): Promise<void> {
  await page.getByTestId('onboarding-next-button').click();
}

async function waitForOnboardingRoute(page: Page): Promise<void> {
  await expect
    .poll(async () => page.evaluate(() => window.location.hash))
    .toMatch(/^#\/onboarding\/welcome/);
  await expect(page.getByTestId('onboarding-layout')).toBeVisible();
}

async function signInToOnboarding(page: Page, userId: string): Promise<void> {
  const payload = Buffer.from(
    JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  const token = `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.sig`;
  await callCoreRpc('openhuman.auth_store_session', { token });
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/welcome');
  await waitForAppReady(page);
  await waitForOnboardingRoute(page);
}

async function completeCloudOnboarding(page: Page): Promise<void> {
  await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible();
  await clickOnboardingNext(page);
  await expect(page.getByTestId('onboarding-runtime-choice-step')).toBeVisible();
  await page.getByTestId('onboarding-runtime-choice-cloud').click();
  await clickOnboardingNext(page);
  const reachedHome = await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toMatch(/^#\/home/)
    .then(
      () => true,
      () => false
    );
  if (!reachedHome) {
    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
    await page.goto('/#/home');
    await waitForAppReady(page);
  }
}

async function logoutViaSettings(page: Page): Promise<void> {
  await callCoreRpc('openhuman.auth_clear_session', {});
  await page.goto('/#/');
  await expect(page.getByText('Welcome to OpenHuman')).toBeVisible();
}

test.describe('Logout -> re-login onboarding overlay', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('re-login after logout returns to the first onboarding step with clean state', async ({
    page,
  }) => {
    await signInToOnboarding(page, 'pw-logout-relogin-user');
    await completeCloudOnboarding(page);
    await logoutViaSettings(page);

    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
    await page.goto('/#/');
    await expect(page.getByText('Welcome to OpenHuman')).toBeVisible();

    await signInToOnboarding(page, 'pw-logout-relogin-user');

    await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible();
    await expect(page.getByText("Hi. I'm OpenHuman.")).toBeVisible();
    await expect(page.getByRole('button', { name: 'Get Started' })).toBeVisible();
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/onboarding\/welcome/);
  });
});
