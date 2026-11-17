import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function waitForNotificationsSections(page: Page): Promise<void> {
  await expect(page.getByTestId('integration-notifications-section')).toBeVisible();
  await expect(page.getByTestId('system-events-section')).toBeVisible();
}

test.describe('Notifications', () => {
  test('Notifications page renders integration notifications', async ({ page }) => {
    const title = `PW Notification UI ${Date.now()}`;
    const body = `Created by the notifications Playwright spec ${Date.now()}`;

    await callCoreRpc<{ id?: string; skipped?: boolean }>('openhuman.notification_ingest', {
      provider: 'e2e',
      title,
      body,
      raw_payload: {},
    });

    await bootAuthenticatedPage(page, 'pw-notifications-ui', '/notifications?view=main');
    await dismissWalkthroughIfPresent(page);
    await waitForNotificationsSections(page);

    await expect(page.getByText(title, { exact: true })).toBeVisible();
    await expect(page.getByText(body, { exact: true })).toBeVisible();
  });

  test('Notifications page shows System Events section', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-notifications-system', '/notifications?view=main');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await waitForNotificationsSections(page);

    await expect(page.getByRole('heading', { name: 'Alerts', exact: true })).toBeVisible();
    await expect(page.getByText('No alerts yet').first()).toBeVisible();
  });
});
