import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  signInViaBypassUser,
  signInViaCallbackToken,
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

async function requests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await response.json()) as { data?: MockRequest[] };
  return Array.isArray(payload.data) ? payload.data : [];
}

async function waitForMockRequest(method: string, pathFragment: string, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const match = (await requests()).find(
      request => request.method === method && request.url.includes(pathFragment)
    );
    if (match) return match;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  return null;
}

test.describe('Login Flow', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('callback login consumes the mock login token and lands on home', async ({ page }) => {
    await signInViaCallbackToken(page, 'playwright-login-token');

    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();
  });

  test('bypass login skips token consume and still lands on home', async ({ page }) => {
    await signInViaBypassUser(page, 'playwright-bypass-user');

    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);

    const consumeCall = (await requests()).find(
      request => request.method === 'POST' && request.url.includes('/telegram/login-tokens/')
    );
    expect(consumeCall).toBeUndefined();
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();
  });
});
