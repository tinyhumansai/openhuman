import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

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

test.describe('Auth & Access Control', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('authenticated sign-in reaches home', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-access-token');

    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();
  });

  test('re-authenticating with a second bypass user keeps the user in-app', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-access-first');
    await dismissWalkthroughIfPresent(page);

    await signInViaBypassUser(page, 'pw-auth-access-second');

    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);
    await expect
      .poll(async () => {
        const requests = await mockRequests();
        return requests.filter(
          request => request.method === 'GET' && request.url.includes('/auth/me')
        ).length;
      })
      .toBeGreaterThanOrEqual(2);
  });

  test('second-device bypass token is accepted without hitting token consume', async ({ page }) => {
    test.skip(
      true,
      'shared web auth bootstrap is unstable for a second-device bypass sign-in and can fall back to onboarding instead of home'
    );
  });

  test('billing dashboard handoff remains available for authenticated users', async ({ page }) => {
    test.skip(
      true,
      'shared web auth/bootstrap helper is not stable enough yet for settings->billing coverage in this lane'
    );
  });

  test('logout via settings clears the session and returns to welcome', async ({ page }) => {
    test.skip(
      true,
      'shared web auth/bootstrap helper is not stable enough yet for logout coverage without crashing the standalone core lane'
    );
  });

  test('auth-expired event signs the user out and lands on welcome', async ({ page }) => {
    test.skip(
      true,
      'web Playwright lane uses a local/bypass session that intentionally ignores auth-expired handling'
    );
  });
});
