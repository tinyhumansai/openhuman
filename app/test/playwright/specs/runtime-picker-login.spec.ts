import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
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
  while (Date.now() < deadline) {
    const match = (await mockRequests()).find(
      request => request.method === method && request.url.includes(pathFragment)
    );
    if (match) return match;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  return null;
}

async function openRuntimePicker(page: Page): Promise<void> {
  if (
    await page
      .getByText('Connect to Your Runtime')
      .isVisible()
      .catch(() => false)
  ) {
    return;
  }
  await dismissWalkthroughIfPresent(page);
  await page.getByRole('button', { name: 'Select a Runtime' }).click({ force: true });
  await expect(page.getByText('Connect to Your Runtime')).toBeVisible();
}

test.describe('Runtime picker -> login -> logout', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('runtime picker validates cloud URL/token inputs and unreachable hosts', async ({
    page,
  }) => {
    test.skip(
      true,
      'web Playwright lane does not reliably surface the desktop-style runtime picker overlay yet'
    );
    await openRuntimePicker(page);

    await page.getByText('Run on the Cloud (Complex)').click();
    await expect(page.getByText('Runtime URL')).toBeVisible();
    await expect(page.getByText('Auth Token')).toBeVisible();

    await page.getByRole('button', { name: 'Continue' }).click();
    await expect(page.getByText('Please enter a runtime URL.')).toBeVisible();

    await page.locator('input[type="url"]').fill('http://127.0.0.1:1/rpc');
    await page.getByRole('button', { name: 'Continue' }).click();
    await expect(page.getByText("We'll need an auth token to connect.")).toBeVisible();

    await page.locator('input[type="password"]').fill('bad-token-e2e');
    await page.getByRole('button', { name: 'Test Connection' }).click();
    await expect(
      page.getByText(/Couldn't reach it:|That token didn't work\. Double-check it and try again\./)
    ).toBeVisible({ timeout: 20_000 });
  });

  test('returning to cloud-mode guest state keeps provider login available', async ({ page }) => {
    test.skip(
      true,
      'web Playwright lane does not reliably surface the desktop-style runtime picker overlay yet'
    );
    await openRuntimePicker(page);

    await page.getByText('Run on the Cloud (Complex)').click();
    await page.locator('input[type="url"]').fill('http://127.0.0.1:17788/rpc');
    await page.locator('input[type="password"]').fill('openhuman-playwright-token');
    await page.getByRole('button', { name: 'Continue' }).click();

    await waitForAppReady(page);
    await expect(page.getByText('Welcome to OpenHuman')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Select a Runtime' })).toBeVisible();
  });

  test('provider login reaches home and logout returns to welcome', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-runtime-picker-login');
    await dismissWalkthroughIfPresent(page);

    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();

    await page.goto('/#/settings/account');
    await waitForAppReady(page);
    await page.getByTestId('settings-nav-logout').click();

    await expect(page.getByText('Welcome to OpenHuman')).toBeVisible();
  });
});
