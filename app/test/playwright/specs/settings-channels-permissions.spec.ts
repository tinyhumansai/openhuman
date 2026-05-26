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
    await page.goto('/#/skills');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    const channelsTab = page.getByRole('tab', { name: 'Channels', exact: true });
    if (await channelsTab.isVisible().catch(() => false)) {
      await channelsTab.click();
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

    await expect(page.getByRole('heading', { name: 'Privacy & Security' })).toBeVisible();
    await expect(page.getByText('Anonymized Analytics')).toBeVisible();
    await expect(page.getByText('Share Anonymized Usage Data')).toBeVisible();
    await expect(page.getByText('What leaves your computer')).toBeVisible();
  });
});
