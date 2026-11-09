import { expect, type Page, test, type TestInfo } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function attachCheckpoint(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  await testInfo.attach(`${name}.png`, {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  await testInfo.attach(`${name}.html`, { body: await page.content(), contentType: 'text/html' });
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  expect(response.ok, 'mock request log is available for the checkpoint').toBe(true);
  const payload: unknown = await response.json();
  expect(payload && typeof payload === 'object' && 'data' in payload).toBe(true);
  const requests = (payload as { data: unknown }).data;
  expect(Array.isArray(requests), 'mock request log contains a data array').toBe(true);
  const safeRequests = (requests as Array<Record<string, unknown>>).map(request => {
    const safe = { ...request };
    for (const key of Object.keys(safe)) {
      if (/authorization|token|credential|secret/i.test(key)) safe[key] = '[redacted]';
    }
    return safe;
  });
  await testInfo.attach(`${name}.mock-requests.json`, {
    body: JSON.stringify(safeRequests, null, 2),
    contentType: 'application/json',
  });
}

async function bootReviewedFlow(page: import('@playwright/test').Page, userId: string) {
  await bootRuntimeReadyGuestPage(page);
  try {
    await signInViaBypassUser(page, userId);
  } catch {
    await bootRuntimeReadyGuestPage(page);
    await signInViaBypassUser(page, userId);
  }
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Agent review - canonical shell + privacy flow', () => {
  test('reaches the shell and opens the privacy panel with review artifacts', async ({
    page,
  }, testInfo) => {
    await bootReviewedFlow(page, 'pw-agent-review');

    // The composer is the durable readiness marker for the unified chat shell;
    // sidebar copy and empty-state headings change independently of readiness.
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
    await attachCheckpoint(page, testInfo, '01-shell-ready');

    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);

    await expect(page.getByTestId('settings-privacy-panel')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Product Analytics' })).toBeVisible();
    await expect(page.getByText('Share Product Analytics and Diagnostics')).toBeVisible();
    await expect(page.getByTestId('privacy-analytics-toggle')).toBeVisible();
    await attachCheckpoint(page, testInfo, '02-privacy-panel');
  });
});
