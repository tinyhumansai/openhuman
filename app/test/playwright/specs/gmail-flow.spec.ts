import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

const CONNECTOR_NAME = 'Gmail';
const TOOLKIT_SLUG = 'gmail';
const CONNECTION_ID = 'c-gmail-1';
const ACTION = 'GMAIL_FETCH_EMAILS';
const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

type RequestLogEntry = { method?: string; url?: string; body?: string };

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

async function getRequestLog(): Promise<RequestLogEntry[]> {
  const payload = await mockFetch('/__admin/requests');
  return (payload.data as RequestLogEntry[]) ?? [];
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
    await signInViaCallbackToken(page, userId);
  } catch {
    await bootRuntimeReadyGuestPage(page);
    await signInViaCallbackToken(page, userId);
  }
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    window.location.hash = '/skills';
  });
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const heading = page.getByRole('heading', { name: 'Composio Integrations' });
  if (!(await heading.isVisible().catch(() => false))) {
    const connectionsButton = page.getByRole('button', { name: 'Connections' });
    if (await connectionsButton.isVisible().catch(() => false)) {
      await connectionsButton.click({ force: true });
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
    }
  }
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

    await page.getByTestId('skill-install-composio-gmail').click();
    await expect(page.getByRole('dialog', { name: /Connect Gmail/i })).toBeVisible();
  });

  test('connected Gmail exposes management affordances', async ({ page }) => {
    const dialog = await openModal(page);
    await expect(dialog).toContainText(CONNECTOR_NAME);
    await expect(dialog.getByTestId('trigger-toggles')).toBeVisible();
  });

  test('authorize routes through the mock backend', async () => {
    await callCoreRpc('openhuman.composio_authorize', { toolkit: TOOLKIT_SLUG });
    const requests = await getRequestLog();
    const authReq = requests.find(
      request =>
        request.method === 'POST' && request.url?.includes('/agent-integrations/composio/authorize')
    );
    expect(authReq).toBeDefined();
  });

  test('failed and expired states remain usable', async ({ page }) => {
    await seedConnector('FAILED');
    await page.reload();
    await waitForAppReady(page);
    await expect(page.getByTestId('skill-install-composio-gmail')).toContainText(CONNECTOR_NAME);

    await seedConnector('EXPIRED');
    await page.reload();
    await waitForAppReady(page);
    await expect(page.getByTestId(`skill-install-composio-${TOOLKIT_SLUG}`)).toContainText(
      /Auth expired|Reconnect/i
    );
  });

  test('execute and disconnect routes do not blank the skills page', async ({ page }) => {
    await callCoreRpc('openhuman.composio_execute', { tool: ACTION, arguments: {} });
    await expect(page.getByRole('heading', { name: 'Composio Integrations' })).toBeVisible();

    await callCoreRpc('openhuman.composio_delete_connection', { connection_id: CONNECTION_ID });
    const requests = await getRequestLog();
    const deleteReq = requests.find(
      request =>
        request.method === 'DELETE' &&
        request.url?.includes('/agent-integrations/composio/connections/')
    );
    expect(deleteReq).toBeDefined();
  });
});
