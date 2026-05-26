import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

const CONNECTOR_NAME = 'Airtable';
const TOOLKIT_SLUG = 'airtable';
const CONNECTION_ID = 'c-airtable-1';
const ACTION = 'AIRTABLE_LIST_BASES';
const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

type RequestLogEntry = { method?: string; url?: string; body?: string };

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
  });
  await page.evaluate(() => {
    window.location.hash = '/skills';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
    .toContain('/skills');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const heading = page.getByRole('heading', { name: 'Composio Integrations' });
  if (!(await heading.isVisible().catch(() => false))) {
    const connectionsButton = page.getByRole('button', { name: 'Connections' });
    if (await connectionsButton.isVisible().catch(() => false)) {
      await connectionsButton.click({ force: true });
      await expect
        .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
        .toContain('/skills');
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
    }
  }
  await expect(page.getByRole('heading', { name: 'Composio Integrations' })).toBeVisible({
    timeout: 20_000,
  });
}

async function reloadSkills(page: Page) {
  await ensureComposioSurface(page);
}

async function ensureComposioSurface(page: Page) {
  const heading = page.getByRole('heading', { name: 'Composio Integrations' });
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.evaluate(() => {
      window.location.hash = '/skills';
    });
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/skills');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    if (await heading.isVisible().catch(() => false)) {
      return;
    }
    const connectionsButton = page.getByRole('button', { name: 'Connections' });
    if (await connectionsButton.isVisible().catch(() => false)) {
      await connectionsButton.click({ force: true });
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
      if (await heading.isVisible().catch(() => false)) {
        return;
      }
    }
    await page.waitForTimeout(500);
  }
  await expect(heading).toBeVisible({ timeout: 20_000 });
}

async function assertSessionNotNuked(page: Page) {
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
          hash: window.location.hash,
          hasToken: Boolean(snapshot?.sessionToken),
          hasUser: Boolean(snapshot?.currentUser?._id),
        };
      })
    )
    .toEqual({ hash: '#/skills', hasToken: true, hasUser: true });
}

async function openConnectorModal(page: Page) {
  const tile = page.getByTestId('skill-install-composio-' + TOOLKIT_SLUG);
  await tile.scrollIntoViewIfNeeded();
  await tile.click();
  const dialog = page.getByRole('dialog', {
    name: new RegExp('(Connect|Manage|Reconnect) ' + CONNECTOR_NAME, 'i'),
  });
  await expect(dialog).toBeVisible();
  return dialog;
}

function unwrapConnections(payload: unknown): Array<{ toolkit?: string; status?: string }> {
  const root = payload as {
    result?: { connections?: Array<{ toolkit?: string; status?: string }> };
    connections?: Array<{ toolkit?: string; status?: string }>;
  };
  return root.result?.connections ?? root.connections ?? [];
}

test.describe('Airtable connector', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkillsPage(page, 'pw-airtable-' + testSlug);
  });

  test('renders the connector card', async ({ page }) => {
    await expect(page.getByTestId('skill-install-composio-' + TOOLKIT_SLUG)).toContainText(
      CONNECTOR_NAME
    );
  });

  test('routes authorize through the mock backend', async () => {
    await callCoreRpc('openhuman.composio_authorize', { toolkit: TOOLKIT_SLUG });
    const requests = await getRequestLog();
    const authReq = requests.find(
      request =>
        request.method === 'POST' && request.url?.includes('/agent-integrations/composio/authorize')
    );
    expect(authReq).toBeDefined();
    expect(JSON.parse(authReq?.body || '{}')).toMatchObject({ toolkit: TOOLKIT_SLUG });
  });

  test('persists connected state through list_connections', async () => {
    const payload = await callCoreRpc<unknown>('openhuman.composio_list_connections', {});
    const hit = unwrapConnections(payload).find(
      connection => connection.toolkit?.toLowerCase() === TOOLKIT_SLUG
    );
    expect(hit?.status).toBe('ACTIVE');
  });

  test.skip('keeps the session alive after composio_sync', async ({ page }) => {
    await callCoreRpc('openhuman.composio_sync', { connection_id: CONNECTION_ID });
    await assertSessionNotNuked(page);
  });

  test('routes composio_execute without blanking the app', async ({ page }) => {
    await callCoreRpc('openhuman.composio_execute', { tool: ACTION, arguments: {} });
    await assertSessionNotNuked(page);
  });

  test('survives failed connector state on the skills page', async ({ page }) => {
    await seedConnector('FAILED');
    await reloadSkills(page);
    await expect(page.getByTestId('skill-install-composio-' + TOOLKIT_SLUG)).toContainText(
      CONNECTOR_NAME
    );
    await assertSessionNotNuked(page);
  });

  test('shows expired-auth state without logging out', async ({ page }) => {
    await seedConnector('EXPIRED');
    await reloadSkills(page);
    await expect(page.getByTestId('skill-install-composio-' + TOOLKIT_SLUG)).toContainText(
      /Auth expired|Reconnect/i
    );
    const dialog = await openConnectorModal(page);
    await expect(dialog).toContainText(CONNECTOR_NAME);
    await assertSessionNotNuked(page);
  });

  test('survives a 4xx composio execute error', async ({ page }) => {
    await setMockBehavior({ composioExecuteFails: '400' });
    await expect(
      callCoreRpc('openhuman.composio_execute', {
        connection_id: CONNECTION_ID,
        tool: ACTION,
        arguments: {},
      })
    ).rejects.toThrow(/failed/i);
    await assertSessionNotNuked(page);
  });

  test('routes disconnect through the mock backend', async ({ page }) => {
    await callCoreRpc('openhuman.composio_delete_connection', { connection_id: CONNECTION_ID });
    const requests = await getRequestLog();
    const deleteReq = requests.find(
      request =>
        request.method === 'DELETE' &&
        request.url?.includes('/agent-integrations/composio/connections/')
    );
    expect(deleteReq).toBeDefined();
    await assertSessionNotNuked(page);
  });
});
