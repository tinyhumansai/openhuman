import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
  waitForAppReady,
} from '../helpers/core-rpc';

const CONNECTOR_NAME = 'Gmail';
const TOOLKIT_SLUG = 'gmail';
const CONNECTION_ID = 'c-gmail-1';
const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

async function mockFetch(path: string, init?: RequestInit) {
  const response = await fetch(MOCK_BASE + path, init);
  if (!response.ok) throw new Error('mock request failed: ' + response.status + ' ' + path);
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

async function seedConnector(status: 'ACTIVE' | 'FAILED' | 'EXPIRED' = 'ACTIVE') {
  await setMockBehavior({
    composioToolkits: JSON.stringify([TOOLKIT_SLUG]),
    composioConnections: JSON.stringify([{ id: CONNECTION_ID, toolkit: TOOLKIT_SLUG, status }]),
  });
}

async function bootSkillsPage(page: Page, userId: string) {
  await resetMock();
  await seedConnector();
  await bootRuntimeReadyGuestPage(page);
  try {
    await signInViaBypassUser(page, userId);
  } catch {
    await bootRuntimeReadyGuestPage(page);
    await signInViaBypassUser(page, userId);
  }
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
  });
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  // Navigate through the shell after sign-in: startup route restoration can
  // otherwise replace a hash assigned during the authenticated boot.
  await page.getByRole('button', { name: 'Connections' }).click();
  await expect(page).toHaveURL(/#\/connections/);
  // Tab is "Apps"; the grid renders in the composio-integrations-card container.
  await page.getByTestId('two-pane-nav-composio').click();
  // Wait for the Apps tab grid container to be visible.
  const heading = page.getByTestId('composio-integrations-card');
  await expect(heading).toBeVisible({ timeout: 20_000 });
}

async function openModal(page: Page) {
  await page.getByTestId('skill-install-composio-gmail').click();
  const dialog = page.getByRole('dialog', { name: /(Connect|Manage|Reconnect) Gmail/i });
  await expect(dialog).toBeVisible();
  return dialog;
}

test.describe('Gmail Integration Flows', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkillsPage(page, `pw-gmail-flow-${slug}`);
  });

  test('setup wizard affordance appears in connect mode', async ({ page }) => {
    await setMockBehavior({
      composioToolkits: JSON.stringify([TOOLKIT_SLUG]),
      composioConnections: JSON.stringify([]),
    });
    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByTestId('two-pane-nav-composio').click();

    await page.getByTestId('skill-install-composio-gmail').click();
    const dialog = page.getByRole('dialog', { name: /Connect Gmail/i });
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByTestId('skill-install-composio-gmail')).toHaveAccessibleName(/Connect/i);
  });

  test('connected Gmail exposes management affordances', async ({ page }) => {
    const dialog = await openModal(page);
    await expect(dialog).toContainText(CONNECTOR_NAME);
    await expect(dialog.getByTestId('trigger-toggles')).toBeVisible();
  });

  test('disconnects Gmail from the current Composio management screen', async ({ page }) => {
    const dialog = await openModal(page);
    await expect(dialog).toContainText('Gmail is connected');

    await dialog.getByRole('button', { name: 'Disconnect', exact: true }).click();

    await expect(dialog.getByRole('button', { name: 'Connect Gmail', exact: true })).toBeVisible({
      timeout: 15_000,
    });
    await expect(dialog).not.toContainText('Gmail is connected');
    await dialog.getByRole('button', { name: /close/i }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByTestId('skill-install-composio-gmail')).toHaveAccessibleName(/Connect/i);

    await page.getByTestId('skill-install-composio-gmail').click();
    const setup = page.getByRole('dialog', { name: /Connect Gmail/i });
    await expect(setup).toBeVisible();
    await expect(setup.getByRole('button', { name: 'Connect Gmail', exact: true })).toBeVisible();
  });

  test('failed and expired states remain usable', async ({ page }) => {
    await seedConnector('FAILED');
    await page.reload();
    await waitForAppReady(page);
    await expect(page.getByRole('button', { name: 'Connections' })).toBeVisible();
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByTestId('two-pane-nav-composio').click();
    await expect(page.getByTestId('skill-install-composio-gmail')).toContainText(CONNECTOR_NAME);

    await seedConnector('EXPIRED');
    await page.reload();
    await waitForAppReady(page);
    await expect(page.getByRole('button', { name: 'Connections' })).toBeVisible();
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByTestId('two-pane-nav-composio').click();
    await expect(page.getByTestId(`skill-install-composio-${TOOLKIT_SLUG}`)).toContainText(
      /Auth expired|Reconnect/i
    );
  });
});
