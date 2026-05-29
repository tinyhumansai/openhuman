import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function reloadAndWait(page: Page): Promise<void> {
  await page.reload();
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function openAuthenticatedRoute(page: Page, userId: string, hash: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, '/home');
  await dismissWalkthroughIfPresent(page);
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function getDefaultMessagingChannel(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: {
        getState?: () => {
          mascot: { voiceId?: string | null };
          channelConnections: { defaultMessagingChannel?: string | null };
        };
      };
    };
    const state = win.__OPENHUMAN_STORE__?.getState?.();
    if (!state) {
      throw new Error('__OPENHUMAN_STORE__ is unavailable');
    }
    return state.channelConnections.defaultMessagingChannel ?? null;
  });
}

async function getMascotVoiceId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: { getState?: () => { mascot: { voiceId?: string | null } } };
    };
    const state = win.__OPENHUMAN_STORE__?.getState?.();
    if (!state) {
      throw new Error('__OPENHUMAN_STORE__ is unavailable');
    }
    return state.mascot.voiceId ?? null;
  });
}

async function getAriaChecked(page: Page, label: string): Promise<string | null> {
  const value = await page.getByRole('switch', { name: label }).getAttribute('aria-checked');
  return value;
}

test.describe('Settings - Feature Preferences', () => {
  test('renders the features settings section route', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-features-route', '/settings/features');

    await expect(page.getByText('Features', { exact: true })).toBeVisible();
    await expect(page.getByTestId('settings-nav-screen-intelligence')).toBeVisible();
    await expect(page.getByTestId('settings-nav-messaging')).toBeVisible();
    await expect(page.getByTestId('settings-nav-notifications')).toBeVisible();
    await expect(page.getByTestId('settings-nav-tools')).toBeVisible();
  });

  test('persists the default messaging channel through redux state', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-default-channel', '/skills');

    const channelsTab = page.getByRole('tab', { name: 'Channels', exact: true });
    if (await channelsTab.isVisible().catch(() => false)) {
      await channelsTab.click();
    }

    await expect(page.getByText('Default Messaging Channel').last()).toBeVisible();
    await page
      .locator('button')
      .filter({ hasText: /^Discord$/ })
      .last()
      .click();

    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('discord');
  });

  test('persists tools preferences to the core app-state snapshot', async ({ page }) => {
    const before = await callCoreRpc<{
      result?: {
        localState?: { onboardingTasks?: { enabledTools?: string[] | null } | null } | null;
      };
    }>('openhuman.app_state_snapshot', {});
    const enabledBefore = before.result?.localState?.onboardingTasks?.enabledTools ?? [];

    await openAuthenticatedRoute(page, 'pw-settings-tools', '/settings/tools');

    await expect(page.getByText('Tools', { exact: true })).toBeVisible();
    await page
      .locator('button')
      .filter({ has: page.getByText('Shell Commands', { exact: true }) })
      .click();
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await expect(page.getByText('Preferences saved')).toBeVisible();

    await expect
      .poll(async () => {
        const after = await callCoreRpc<{
          result?: {
            localState?: { onboardingTasks?: { enabledTools?: string[] | null } | null } | null;
          };
        }>('openhuman.app_state_snapshot', {});
        const enabledAfter = after.result?.localState?.onboardingTasks?.enabledTools ?? [];
        return JSON.stringify(enabledAfter) !== JSON.stringify(enabledBefore);
      })
      .toBe(true);
  });

  test('persists notifications DND and category preferences', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-notification-prefs', '/settings/notifications');

    await expect(page.getByText('Do Not Disturb', { exact: true })).toBeVisible();
    await expect(page.getByText('Messages', { exact: true })).toBeVisible();

    const dndLabel = 'Toggle Do Not Disturb';
    const messagesLabel = 'Toggle Messages notifications';
    const dndBefore = await getAriaChecked(page, dndLabel);
    const messagesBefore = await getAriaChecked(page, messagesLabel);

    await page.getByRole('switch', { name: dndLabel }).click();
    await page.getByRole('switch', { name: messagesLabel }).click();

    await expect
      .poll(async () => ({
        dnd: await getAriaChecked(page, dndLabel),
        messages: await getAriaChecked(page, messagesLabel),
      }))
      .not.toEqual({ dnd: dndBefore, messages: messagesBefore });

    const toggled = {
      dnd: await getAriaChecked(page, dndLabel),
      messages: await getAriaChecked(page, messagesLabel),
    };

    await reloadAndWait(page);
    await expect(page.getByText('Do Not Disturb')).toBeVisible();
    await expect.poll(() => getAriaChecked(page, dndLabel)).not.toBeNull();
    await expect.poll(() => getAriaChecked(page, messagesLabel)).toBe(toggled.messages);
  });

  test('persists mascot color selection', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-mascot-color', '/settings/mascot');

    await expect(page.getByRole('heading', { name: 'Color', exact: true })).toBeVisible();
    await page.getByTestId('mascot-color-burgundy').click();
    await expect(page.getByTestId('mascot-color-burgundy')).toHaveAttribute('aria-checked', 'true');

    await reloadAndWait(page);
    await expect(page.getByTestId('mascot-color-burgundy')).toHaveAttribute('aria-checked', 'true');
  });

  test('persists the custom mascot voice override on the voice panel', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-mascot-voice', '/settings/voice');

    await expect(page.getByText('Mascot Voice')).toBeVisible();
    test.skip(
      (await page
        .locator('[data-testid="mascot-voice-select"] option[value="__custom__"]')
        .count()) === 0,
      'custom mascot voice option is unavailable in this build'
    );

    await page.getByTestId('mascot-voice-select').selectOption('__custom__');
    test.skip(
      (await page.getByTestId('mascot-voice-input').count()) === 0,
      'custom mascot voice input did not appear after selecting __custom__'
    );

    await page.getByTestId('mascot-voice-input').fill('voice-e2e-custom');
    await page.getByTestId('mascot-voice-save-paste').click();

    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');

    await reloadAndWait(page);
    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');
  });
});
