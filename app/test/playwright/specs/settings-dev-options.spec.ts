import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Settings - Developer Options', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-dev-user');
  });

  test('mounts Webhooks Debug panel', async ({ page }) => {
    await page.goto('/#/settings/webhooks-debug');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Webhooks Debug')).toBeVisible();
    await expect(page.getByText('Registered Webhooks')).toBeVisible();
    await expect(page.getByText('Captured Requests')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Refresh' }).first()).toBeVisible();
  });

  test('mounts Memory Debug panel', async ({ page }) => {
    await page.goto('/#/settings/memory-debug');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Memory Debug')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Documents', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Namespaces', exact: true })).toBeVisible();
    await expect(page.getByText('Query & Recall')).toBeVisible();
    await expect(page.getByText('Clear Namespace')).toBeVisible();
  });

  test('shows Live Logs in Autocomplete Debug panel', async ({ page }) => {
    await page.goto('/#/settings/autocomplete-debug');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Autocomplete Debug')).toBeVisible();
    await expect(page.getByText('Live Logs')).toBeVisible();
    await expect(page.getByText(/No logs yet\.|\[runtime\]/)).toBeVisible();
  });
});
