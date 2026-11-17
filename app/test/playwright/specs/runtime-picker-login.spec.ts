import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaBypassUserPreservingConfig,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

interface MockRequest {
  method: string;
  url: string;
}

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function mockRequests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await response.json()) as { data?: MockRequest[] };
  return Array.isArray(payload.data) ? payload.data : [];
}

async function waitForMockRequest(method: string, pathFragment: string, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let delay = 200;
  while (Date.now() < deadline) {
    const match = (await mockRequests()).find(
      request => request.method === method && request.url.includes(pathFragment)
    );
    if (match) return match;
    await new Promise(resolve => setTimeout(resolve, delay));
    delay = Math.min(delay * 1.5, 1_000);
  }
  return null;
}

async function readOnboardingCompleted(): Promise<boolean> {
  const response = await callCoreRpc<boolean | { result?: boolean }>(
    'openhuman.config_get_onboarding_completed',
    {}
  );
  const value = typeof response === 'boolean' ? response : response?.result;
  expect(typeof value).toBe('boolean');
  return value as boolean;
}

test.describe('Runtime picker -> login -> logout', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('shows current welcome choices and provider sign-in options', async ({ page }) => {
    await expect(page.getByTestId('welcome-card-self')).toBeVisible();
    await expect(page.getByTestId('welcome-cta-self')).toBeVisible();
    await expect(page.getByTestId('welcome-cta-tinyhumans')).toBeVisible();
    await expect(
      page.getByRole('button', { name: /Google|GitHub|Twitter|Discord/i }).first()
    ).toBeVisible();
  });

  test('self-hosted choice completes the current custom setup wizard', async ({ page }) => {
    await page.getByTestId('welcome-cta-self').click();
    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible();
    await page.getByTestId('onboarding-next-button').click();
    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible();
    await page.getByTestId('onboarding-search-skip').click();
    await expect(page.getByTestId('onboarding-custom-vault-step')).toBeVisible();
    await page.getByTestId('onboarding-next-button').click();
    await expect.poll(() => page.evaluate(() => window.location.hash)).toContain('#/chat');
    await expect.poll(readOnboardingCompleted).toBe(true);
  });

  test('provider login reaches home and logout returns to welcome', async ({ page }) => {
    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
    await signInViaBypassUserPreservingConfig(page, 'pw-runtime-picker-login');
    await dismissWalkthroughIfPresent(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/(home|chat)(\/|$)/);
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();
    await expect.poll(readOnboardingCompleted).toBe(true);

    await page.goto('/#/settings/account');
    await waitForAppReady(page);
    await page.getByTestId('settings-nav-logout').click();

    await expect(page.getByText('Welcome to OpenHuman')).toBeVisible();
    await expect(page.getByTestId('welcome-cta-self')).toBeVisible();
    await expect(page.getByTestId('welcome-cta-tinyhumans')).toBeVisible();
  });
});
