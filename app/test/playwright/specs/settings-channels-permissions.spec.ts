import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function getDefaultMessagingChannel(
  page: import('@playwright/test').Page
): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: {
        getState?: () => { channelConnections?: { defaultMessagingChannel?: string | null } };
      };
    };
    return (
      win.__OPENHUMAN_STORE__?.getState?.().channelConnections?.defaultMessagingChannel ?? null
    );
  });
}

test.describe('Settings - Channels & Permissions', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-channels-user');
  });

  test('allows switching default messaging channel', async ({ page }) => {
    // Phase 2: default messaging channel UI moved to /connections (Messaging tab)
    await page.goto('/#/connections?tab=messaging');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    const messagingTab = page.getByTestId('two-pane-nav-channels');
    if (await messagingTab.isVisible().catch(() => false)) {
      await messagingTab.click();
    }

    await expect(page.getByText('Default Messaging Channel').last()).toBeVisible();
    await expect(page.getByText('Telegram').last()).toBeVisible();
    await expect(page.getByText('Discord').last()).toBeVisible();

    await page.getByText('Discord').last().click();
    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('discord');
  });

  test('renders privacy settings and analytics toggle', async ({ page }) => {
    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByTestId('settings-privacy-panel')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Product Analytics' })).toBeVisible();
    await expect(page.getByText('Share Product Analytics and Diagnostics')).toBeVisible();
    await expect(page.getByText('What leaves your computer')).toBeVisible();
  });
});
