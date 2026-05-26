import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

const TOOLKIT_SLUG = 'gmail';
const TOOLKIT_NAME = 'Gmail';
const CONNECTION_ID = 'c1';
const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');

type ActiveTrigger = {
  id?: string;
  slug?: string;
  toolkit?: string;
  connectionId?: string;
  connection_id?: string;
};

type EnableTriggerResult = {
  triggerId?: string;
  trigger_id?: string;
  slug?: string;
  connectionId?: string;
  connection_id?: string;
};

type DisableTriggerResult = { deleted?: boolean };

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
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
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
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
    }
  }
  await expect(heading).toBeVisible({ timeout: 20_000 });
}

async function openGmailManageModal(page: Page) {
  await page.getByTestId('skill-install-composio-gmail').click();
  const dialog = page.getByRole('dialog', { name: /(Connect|Manage|Reconnect) Gmail/i });
  await expect(dialog).toBeVisible();
  return dialog;
}

function unwrapTriggers(payload: unknown): ActiveTrigger[] {
  const root = payload as { result?: { triggers?: ActiveTrigger[] }; triggers?: ActiveTrigger[] };
  return root.result?.triggers ?? root.triggers ?? [];
}

function unwrapEnableTrigger(payload: unknown): EnableTriggerResult {
  const root = payload as { result?: EnableTriggerResult } & EnableTriggerResult;
  return root.result ?? root;
}

function unwrapDisableTrigger(payload: unknown): DisableTriggerResult {
  const root = payload as { result?: DisableTriggerResult } & DisableTriggerResult;
  return root.result ?? root;
}

test.describe('Composio triggers flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkillsPage(page, 'pw-composio-triggers-' + testSlug);
  });

  test('list_available_triggers returns the seeded Gmail catalog', async () => {
    const payload = await callCoreRpc<unknown>('openhuman.composio_list_available_triggers', {
      toolkit: TOOLKIT_SLUG,
      connection_id: CONNECTION_ID,
    });
    const triggers = unwrapTriggers(payload);
    const slugs = triggers.map(trigger => trigger.slug);
    expect(slugs).toContain('GMAIL_NEW_GMAIL_MESSAGE');
    expect(slugs).toContain('SLACK_NEW_MESSAGE');
  });

  test('list_triggers starts empty for the seeded user', async () => {
    const payload = await callCoreRpc<unknown>('openhuman.composio_list_triggers', {});
    expect(unwrapTriggers(payload)).toHaveLength(0);
  });

  test('enable_trigger creates a trigger that list_triggers observes', async () => {
    const created = unwrapEnableTrigger(
      await callCoreRpc<unknown>('openhuman.composio_enable_trigger', {
        connection_id: CONNECTION_ID,
        slug: 'GMAIL_NEW_GMAIL_MESSAGE',
      })
    );
    expect(created.slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
    expect(created.connectionId ?? created.connection_id).toBe(CONNECTION_ID);
    expect((created.triggerId ?? created.trigger_id)?.length).toBeGreaterThan(0);

    const listed = await callCoreRpc<unknown>('openhuman.composio_list_triggers', {
      toolkit: TOOLKIT_SLUG,
    });
    const triggers = unwrapTriggers(listed);
    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
  });

  test('disable_trigger removes the active trigger', async () => {
    const created = unwrapEnableTrigger(
      await callCoreRpc<unknown>('openhuman.composio_enable_trigger', {
        connection_id: CONNECTION_ID,
        slug: 'GMAIL_NEW_GMAIL_MESSAGE',
      })
    );
    const triggerId = created.triggerId ?? created.trigger_id;
    expect(triggerId).toBeTruthy();

    const disabled = unwrapDisableTrigger(
      await callCoreRpc<unknown>('openhuman.composio_disable_trigger', { trigger_id: triggerId })
    );
    expect(disabled.deleted).toBe(true);

    const listed = await callCoreRpc<unknown>('openhuman.composio_list_triggers', {});
    expect(unwrapTriggers(listed)).toHaveLength(0);
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
    await expect(page.getByRole('heading', { name: 'Composio Integrations' })).toBeVisible({
      timeout: 20_000,
    });

    const dialog = await openGmailManageModal(page);
    await expect(dialog.getByTestId('trigger-toggles')).toBeVisible();
    await expect(dialog.getByText(/New Gmail Message/)).toBeVisible();
  });
});
