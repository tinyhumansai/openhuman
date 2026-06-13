import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

function getUnreadCount(stats: Record<string, unknown>): number {
  for (const key of ['unread_count', 'unread', 'total_unread']) {
    const value = stats[key];
    if (typeof value === 'number') return value;
  }
  return 0;
}

async function waitForNotificationsSections(page: Page): Promise<void> {
  await expect(page.getByTestId('integration-notifications-section')).toBeVisible();
  await expect(page.getByTestId('system-events-section')).toBeVisible();
}

test.describe('Notifications', () => {
  test('notification_ingest creates a new notification via core RPC', async () => {
    const payload = await callCoreRpc<{ id?: string; skipped?: boolean }>(
      'openhuman.notification_ingest',
      {
        provider: 'e2e',
        title: 'E2E Test Notification',
        body: 'Created by the notifications Playwright spec',
        raw_payload: {},
      }
    );

    expect(payload.skipped).not.toBe(true);
    expect(typeof payload.id).toBe('string');
  });

  test('notification_list returns the ingested notification', async () => {
    const title = `PW Notification List ${Date.now()}`;
    await callCoreRpc<{ id?: string; skipped?: boolean }>('openhuman.notification_ingest', {
      provider: 'e2e',
      title,
      body: 'List coverage notification',
      raw_payload: {},
    });

    const result = await callCoreRpc<{ items?: Array<{ title?: string }> }>(
      'openhuman.notification_list',
      { limit: 20 }
    );

    expect(result.items?.some(item => item.title === title)).toBe(true);
  });

  test('notification_mark_read transitions notification status', async () => {
    const before = await callCoreRpc<Record<string, unknown>>('openhuman.notification_stats', {});
    const initialUnread = getUnreadCount(before);

    const created = await callCoreRpc<{ id: string }>('openhuman.notification_ingest', {
      provider: 'e2e',
      title: `PW Notification Mark Read ${Date.now()}`,
      body: 'Mark read coverage notification',
      raw_payload: {},
    });

    await callCoreRpc('openhuman.notification_mark_read', { id: created.id });

    await expect
      .poll(async () => {
        const after = await callCoreRpc<Record<string, unknown>>(
          'openhuman.notification_stats',
          {}
        );
        return getUnreadCount(after);
      })
      .toBeLessThanOrEqual(initialUnread);
  });

  test('notification_stats returns aggregate statistics', async () => {
    const stats = await callCoreRpc<Record<string, unknown>>('openhuman.notification_stats', {});
    expect(Object.values(stats).some(value => typeof value === 'number')).toBe(true);
  });

  test('Notifications page renders integration notifications', async ({ page }) => {
    const title = `PW Notification UI ${Date.now()}`;
    const body = `Created by the notifications Playwright spec ${Date.now()}`;

    await callCoreRpc<{ id?: string; skipped?: boolean }>('openhuman.notification_ingest', {
      provider: 'e2e',
      title,
      body,
      raw_payload: {},
    });

    await bootAuthenticatedPage(page, 'pw-notifications-ui', '/notifications');
    await dismissWalkthroughIfPresent(page);
    await waitForNotificationsSections(page);

    await expect(page.getByText(title, { exact: true })).toBeVisible();
    await expect(page.getByText(body, { exact: true })).toBeVisible();
  });

  test('Notifications page shows System Events section', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-notifications-system', '/notifications');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await waitForNotificationsSections(page);

    await expect(page.getByRole('heading', { name: 'Alerts', exact: true })).toBeVisible();
    await expect(page.getByText('No alerts yet').first()).toBeVisible();
  });

  test('native notification permission command returns a valid state', async () => {
    test.skip(
      true,
      'web Playwright lane does not expose the Tauri invoke bridge used by the WDIO shell test'
    );
  });
});
