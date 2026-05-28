import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Channels Smoke', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-channels-user', '/channels');
  });

  test('renders Telegram and Discord panels in not-connected state', async ({ page }) => {
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Channels')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Telegram', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: /Telegram Disconnected/ })).toBeVisible();
    await expect(page.getByRole('button', { name: /Discord Disconnected/ })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Connect' }).first()).toBeVisible();
  });
});
