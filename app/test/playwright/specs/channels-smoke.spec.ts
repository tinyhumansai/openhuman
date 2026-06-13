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

    // /channels redirects to /connections?tab=messaging; connectors now render
    // as ChannelTile buttons named "<Name>, <status>. <cta>." (not headings/
    // standalone Disconnected buttons).
    await expect(page.getByRole('button', { name: /Telegram/ }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: /Discord/ }).first()).toBeVisible();
  });
});
