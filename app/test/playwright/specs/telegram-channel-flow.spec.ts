import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

const MOCK_BASE = 'http://127.0.0.1:' + (process.env.E2E_MOCK_PORT || '18473');
const BOT_TOKEN = 'e2e-bot-token-12345:AAFakeTokenForE2E';
const BOT_TOKEN_2 = 'e2e-bot-token-99999:AASecondFakeTokenForE2E';
const BOT_USERNAME = 'e2e_test_bot';

type TelegramStatusEntry = {
  channelId?: string;
  channel_id?: string;
  authMode?: string;
  auth_mode?: string;
  connected?: boolean;
  hasCredentials?: boolean;
  has_credentials?: boolean;
};

async function mockFetch(path: string, init?: RequestInit) {
  const response = await fetch(MOCK_BASE + path, init);
  if (!response.ok) throw new Error('mock request failed: ' + response.status + ' ' + path);
  return response.json() as Promise<{ data?: unknown }>;
}

async function resetTelegramMock() {
  await mockFetch('/__admin/telegram/reset', { method: 'POST' });
}

async function setMockBehavior(behavior: Record<string, unknown>) {
  await mockFetch('/__admin/behavior', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ behavior }),
  });
}

async function connectTelegramBot(opts: {
  botToken: string;
  allowedUsers?: string[];
  mentionOnly?: boolean;
}) {
  const credentials: Record<string, unknown> = { bot_token: opts.botToken };
  if (opts.allowedUsers) credentials.allowed_users = opts.allowedUsers;
  if (opts.mentionOnly !== undefined) credentials.mention_only = opts.mentionOnly;
  return callCoreRpc<{
    result?: { status?: string; restart_required?: boolean; message?: string };
    status?: string;
    restart_required?: boolean;
    message?: string;
  }>('openhuman.channels_connect', { channel: 'telegram', authMode: 'bot_token', credentials });
}

async function disconnectTelegramBot() {
  return callCoreRpc<unknown>('openhuman.channels_disconnect', {
    channel: 'telegram',
    authMode: 'bot_token',
  });
}

async function getTelegramChannelStatus(): Promise<TelegramStatusEntry | null> {
  const out = await callCoreRpc<unknown>('openhuman.channels_status', { channel: 'telegram' });
  const root = (out ?? {}) as Record<string, unknown>;
  const entries = Array.isArray(root)
    ? root
    : Array.isArray(root.entries)
      ? (root.entries as unknown[])
      : Array.isArray(root.result)
        ? (root.result as unknown[])
        : [];
  const match = entries.find(entry => {
    const record = entry as TelegramStatusEntry;
    const channelId = record.channelId ?? record.channel_id;
    const authMode = record.authMode ?? record.auth_mode;
    return channelId === 'telegram' && authMode === 'bot_token';
  });
  return (match as TelegramStatusEntry | undefined) ?? null;
}

test.describe('Telegram channel - connect / disconnect RPC flow', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-telegram-channel-flow', '/home');
    await setMockBehavior({ telegramBotUsername: BOT_USERNAME, telegramPollDelayMs: '0' });
    await resetTelegramMock();
  });

  test('channels_list includes telegram with bot_token auth mode', async () => {
    const out = await callCoreRpc<unknown>('openhuman.channels_list', {});
    const root = (out ?? {}) as Record<string, unknown>;
    const channels = Array.isArray(root)
      ? root
      : Array.isArray(root.channels)
        ? (root.channels as Array<Record<string, unknown>>)
        : Array.isArray(root.result)
          ? (root.result as Array<Record<string, unknown>>)
          : [];
    const telegram = channels.find(channel => channel?.id === 'telegram');
    expect(telegram).toBeDefined();
    const authModes = Array.isArray(telegram?.auth_modes)
      ? (telegram?.auth_modes as unknown[])
      : Array.isArray(telegram?.authModes)
        ? (telegram?.authModes as unknown[])
        : [];
    const hasBotToken = authModes.some(
      mode => (mode as Record<string, unknown>).mode === 'bot_token' || mode === 'bot_token'
    );
    expect(hasBotToken).toBe(true);
  });

  test('channels_describe for telegram returns auth modes and bot_token field', async () => {
    const out = await callCoreRpc<unknown>('openhuman.channels_describe', { channel: 'telegram' });
    const root = (out ?? {}) as Record<string, unknown>;
    const def =
      typeof root.result === 'object' && root.result !== null
        ? (root.result as Record<string, unknown>)
        : typeof root.definition === 'object' && root.definition !== null
          ? (root.definition as Record<string, unknown>)
          : root;

    expect(def.id ?? def.channel_id).toBe('telegram');
    const authModes = Array.isArray(def.auth_modes) ? (def.auth_modes as unknown[]) : [];
    const botTokenSpec = authModes.find(
      mode => (mode as Record<string, unknown>).mode === 'bot_token'
    ) as Record<string, unknown> | undefined;
    expect(botTokenSpec).toBeDefined();
    const fields = Array.isArray(botTokenSpec?.fields) ? (botTokenSpec?.fields as unknown[]) : [];
    expect(fields.some(field => (field as Record<string, unknown>).key === 'bot_token')).toBe(true);
  });

  test('bot-token connect happy path stores credentials and status shows connected', async () => {
    const connectResult = await connectTelegramBot({ botToken: BOT_TOKEN });
    const payload =
      typeof connectResult.result === 'object' && connectResult.result !== null
        ? connectResult.result
        : connectResult;
    expect(payload.status).toBe('connected');
    expect(payload.restart_required).toBe(true);

    const status = await getTelegramChannelStatus();
    expect(status).not.toBeNull();
    expect(status?.connected).toBe(true);
    expect(status?.hasCredentials ?? status?.has_credentials).toBe(true);
  });

  test('connect with missing token fails validation', async () => {
    await expect(
      callCoreRpc('openhuman.channels_connect', {
        channel: 'telegram',
        authMode: 'bot_token',
        credentials: { bot_token: '' },
      })
    ).rejects.toThrow();
  });

  test('disconnect clears channel status', async () => {
    await connectTelegramBot({ botToken: BOT_TOKEN });
    const beforeStatus = await getTelegramChannelStatus();
    expect(beforeStatus?.connected).toBe(true);

    await disconnectTelegramBot();
    const afterStatus = await getTelegramChannelStatus();
    expect(afterStatus === null || afterStatus.connected === false).toBe(true);
  });

  test('reconnect after disconnect succeeds', async () => {
    await connectTelegramBot({ botToken: BOT_TOKEN });
    await disconnectTelegramBot();
    const reconnect = await connectTelegramBot({ botToken: BOT_TOKEN_2 });
    const payload =
      typeof reconnect.result === 'object' && reconnect.result !== null
        ? reconnect.result
        : reconnect;
    expect(payload.status).toBe('connected');

    const status = await getTelegramChannelStatus();
    expect(status?.connected).toBe(true);
    expect(status?.hasCredentials ?? status?.has_credentials).toBe(true);
  });

  test.skip('inbound message polling scenarios require a live listener restart in this lane', async () => {});
});
