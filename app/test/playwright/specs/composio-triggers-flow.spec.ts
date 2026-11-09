import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const TOOLKIT_SLUG = 'gmail';
const CONNECTION_ID = 'c1';
const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

async function mockFetch(path: string, init?: RequestInit) {
  const response = await fetch(MOCK_BASE + path, init);
  if (!response.ok) {
    throw new Error('mock request failed: ' + response.status + ' ' + path);
  }
  return response.json() as Promise<{ data?: unknown }>;
}

async function resetMock() {
  await mockFetch('/__admin/reset', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keepBehavior: false, keepRequests: false }),
  });
}

async function setMockBehavior(behavior: Record<string, unknown>) {
  await mockFetch('/__admin/behavior', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ behavior }),
  });
}

async function bootSkillsPage(page: Page, userId: string) {
  await resetMock();
  await setMockBehavior({
    composioConnections: JSON.stringify([
      { id: CONNECTION_ID, toolkit: TOOLKIT_SLUG, status: 'ACTIVE' },
    ]),
    composioAvailableTriggers: JSON.stringify([
      { slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' },
      { slug: 'SLACK_NEW_MESSAGE', scope: 'static', requiredConfigKeys: ['channel'] },
    ]),
    composioActiveTriggers: JSON.stringify([]),
  });
  await bootAuthenticatedPage(page, userId, '/connections?tab=composio');
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    window.location.hash = '/connections?tab=composio';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
    .toContain('/connections');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  // Tab is "Apps"; the grid renders in the composio-integrations-card container.
  await expect(page.getByTestId('composio-integrations-card')).toBeVisible({ timeout: 20_000 });
}

async function openGmailManageModal(page: Page) {
  await page.getByTestId('skill-install-composio-gmail').click();
  const dialog = page.getByRole('dialog', { name: /(Connect|Manage|Reconnect) Gmail/i });
  await expect(dialog).toBeVisible();
  return dialog;
}

test.describe('Composio trigger section UI', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkillsPage(page, 'pw-composio-triggers-' + testSlug);
  });

  test('renders the Triggers section in the Gmail modal', async ({ page }) => {
    await setMockBehavior({
      composioActiveTriggers: JSON.stringify([
        {
          id: 'ti-seeded',
          slug: 'GMAIL_NEW_GMAIL_MESSAGE',
          toolkit: TOOLKIT_SLUG,
          connectionId: CONNECTION_ID,
        },
      ]),
    });
    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    // Tab is "Apps"; the grid renders in the composio-integrations-card container.
    await page.getByTestId('two-pane-nav-composio').click();
    await expect(page.getByTestId('composio-integrations-card')).toBeVisible({ timeout: 20_000 });

    const dialog = await openGmailManageModal(page);
    await expect(dialog.getByTestId('trigger-toggles')).toBeVisible();
    await expect(dialog.getByText(/New Gmail Message/)).toBeVisible();
  });
});
