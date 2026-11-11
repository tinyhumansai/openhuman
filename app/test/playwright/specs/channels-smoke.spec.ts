import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

type TelegramStatus = {
  channel_id?: string;
  channel?: string;
  auth_mode?: string;
  authMode?: string;
  connected?: boolean;
};

async function telegramConnected(): Promise<boolean> {
  const response = (await callCoreRpc<unknown>('openhuman.channels_status', {
    channel: 'telegram',
  })) as { result?: unknown } | unknown;
  const unwrapped =
    response && typeof response === 'object' && 'result' in response
      ? (response as { result: unknown }).result
      : response;
  const entries = Array.isArray(unwrapped)
    ? unwrapped
    : unwrapped && typeof unwrapped === 'object' && 'entries' in unwrapped
      ? (unwrapped as { entries: unknown }).entries
      : unwrapped;
  expect(Array.isArray(entries), 'channels_status returns channel status entries').toBe(true);
  const telegram = (entries as TelegramStatus[]).find(
    entry =>
      (entry.channel_id ?? entry.channel) === 'telegram' &&
      (entry.auth_mode ?? entry.authMode) === 'bot_token'
  );
  expect(telegram, 'channels_status includes Telegram').toBeDefined();
  const connected = telegram?.connected;
  expect(typeof connected, 'Telegram status includes a boolean connected field').toBe('boolean');
  return connected as boolean;
}

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
    await expect(page.getByRole('button', { name: /Telegram, Not configured/ })).toBeVisible();
    await expect(page.getByRole('button', { name: /Discord/ }).first()).toBeVisible();
    await expect.poll(telegramConnected).toBe(false);
  });

  test('Telegram badge follows live status through connect and disconnect', async ({ page }) => {
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    const telegram = page.getByTestId('skill-row-channel-telegram');

    await callCoreRpc('openhuman.channels_disconnect', {
      channel: 'telegram',
      authMode: 'bot_token',
    });
    await expect.poll(telegramConnected).toBe(false);
    await expect(telegram).toContainText('Not configured');
    await callCoreRpc('openhuman.channels_connect', {
      channel: 'telegram',
      authMode: 'bot_token',
      credentials: { bot_token: '111111:e2e-channels-smoke-token' },
    });
    await expect.poll(telegramConnected).toBe(true);
    await page.goto('/#/chat');
    await page.goto('/#/channels');
    await waitForAppReady(page);
    await expect(telegram).toContainText('Connected');

    await callCoreRpc('openhuman.channels_disconnect', {
      channel: 'telegram',
      authMode: 'bot_token',
    });
    await expect.poll(telegramConnected).toBe(false);
    await page.goto('/#/chat');
    await page.goto('/#/channels');
    await waitForAppReady(page);
    await expect(telegram).toContainText('Not configured');
  });
});
