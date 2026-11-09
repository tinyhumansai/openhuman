import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
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
  // Connector behavior does not exercise the auth callback. Seed the core
  // session directly so callback timing cannot obscure connector failures.
  await bootAuthenticatedPage(page, userId, '/connections?tab=composio');
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
  });
  await page.evaluate(() => {
    window.location.hash = '/connections?tab=composio';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
    .toContain('/connections');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const heading = page.getByTestId('composio-integrations-card');
  if (!(await heading.isVisible().catch(() => false))) {
    const connectionsButton = page.getByRole('button', { name: 'Connections' });
    if (await connectionsButton.isVisible().catch(() => false)) {
      await connectionsButton.click({ force: true });
      await expect
        .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
        .toContain('/connections');
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
    }
  }
  await expect(page.getByTestId('composio-integrations-card')).toBeVisible({ timeout: 20_000 });
}

async function reloadSkills(page: Page) {
  await ensureComposioSurface(page);
}

async function ensureComposioSurface(page: Page) {
  // Navigate to /connections and click the Composio tab
  const heading = page.getByTestId('composio-integrations-card');
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.evaluate(() => {
      window.location.hash = '/connections';
    });
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/connections');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await page.getByTestId('two-pane-nav-composio').click();
    if (await heading.isVisible().catch(() => false)) {
      return;
    }
    const connectionsButton = page.getByRole('button', { name: 'Connections' });
    if (await connectionsButton.isVisible().catch(() => false)) {
      await connectionsButton.click({ force: true });
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
      await page.getByTestId('two-pane-nav-composio').click();
      if (await heading.isVisible().catch(() => false)) {
        return;
      }
    }
  }
  await expect(heading).toBeVisible({ timeout: 20_000 });
}

async function assertSessionNotNuked(page: Page) {
  // Phase 2: /skills → /connections
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const win = window as typeof window & {
          __OPENHUMAN_CORE_STATE__?: () => {
            snapshot?: {
              sessionToken?: string | null;
              currentUser?: { _id?: string | null } | null;
            };
          };
        };
        const snapshot = win.__OPENHUMAN_CORE_STATE__?.()?.snapshot;
        return {
          // The connections page now appends an active-tab query (e.g.
          // `#/connections?tab=composio`); strip it so we assert we're still on
          // the connections route (session not nuked), not the exact sub-tab.
          hash: window.location.hash.replace(/\?.*$/, ''),
          hasToken: Boolean(snapshot?.sessionToken),
          hasUser: Boolean(snapshot?.currentUser?._id),
        };
      })
    )
    .toEqual({ hash: '#/connections', hasToken: true, hasUser: true });
}

async function openModal(page: Page) {
  await page.getByTestId('skill-install-composio-gmail').click();
  const dialog = page.getByRole('dialog', { name: /(Connect|Manage|Reconnect) Gmail/i });
  await expect(dialog).toBeVisible();
  return dialog;
}

test.describe('Gmail connector', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkillsPage(page, 'pw-gmail-' + testSlug);
  });

  test('renders the connector card', async ({ page }) => {
    await expect(page.getByTestId('skill-install-composio-gmail')).toContainText(CONNECTOR_NAME);
  });

  test('survives failed connector state on the skills page', async ({ page }) => {
    await seedConnector('FAILED');
    await reloadSkills(page);
    await expect(page.getByTestId('skill-install-composio-gmail')).toContainText(CONNECTOR_NAME);
    await assertSessionNotNuked(page);
  });

  test('shows expired-auth state without logging out', async ({ page }) => {
    await seedConnector('EXPIRED');
    await reloadSkills(page);
    await expect(page.getByTestId('skill-install-composio-' + TOOLKIT_SLUG)).toContainText(
      /Auth expired|Reconnect/i
    );
    const dialog = await openModal(page);
    await expect(dialog).toContainText(CONNECTOR_NAME);
    await assertSessionNotNuked(page);
  });
});
