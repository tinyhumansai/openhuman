import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-wallet-flow';
const CANARY = 'wallet-quote-canary-8d13';
const JOHN_ADDRESS = '0x00000000000000000000000000000000000000aa';
const WALLET_PROMPT = `Send John $5 on EVM at ${JOHN_ADDRESS} and tell me ${CANARY}.`;
const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_delegate_do_crypto_1',
        name: 'delegate_do_crypto',
        arguments: JSON.stringify({
          prompt: `Prepare a $5 EVM transfer to John at ${JOHN_ADDRESS}.`,
        }),
      },
    ],
  },
  {
    content: '',
    toolCalls: [{ id: 'call_wallet_status_1', name: 'wallet_status', arguments: '{}' }],
  },
  {
    content: '',
    toolCalls: [{ id: 'call_wallet_chain_status_1', name: 'wallet_chain_status', arguments: '{}' }],
  },
  {
    content: '',
    toolCalls: [
      {
        id: 'call_wallet_prepare_transfer_1',
        name: 'wallet_prepare_transfer',
        arguments: JSON.stringify({
          chain: 'evm',
          toAddress: JOHN_ADDRESS,
          amountRaw: '5000000000000000000',
        }),
      },
    ],
  },
  { content: `Prepared a wallet quote for John. ${CANARY}` },
  { content: `Done. ${CANARY}` },
];

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

async function emulateTauriRuntime(page: Page): Promise<void> {
  await page.evaluate(() => {
    const win = window as typeof window & {
      isTauri?: boolean;
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    win.isTauri = true;
    win.__TAURI_INTERNALS__ = win.__TAURI_INTERNALS__ ?? {};
    win.__TAURI_INTERNALS__.invoke = win.__TAURI_INTERNALS__.invoke ?? (async () => null);
  });
}

async function openChat(page: Page): Promise<void> {
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeVisible();
}

async function selectedThreadId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => { thread?: { selectedThreadId?: string | null } };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store?.getState?.().thread?.selectedThreadId ?? null;
  });
}

async function createNewThread(page: Page): Promise<string> {
  const before = await selectedThreadId(page);
  await dismissWalkthroughIfPresent(page);
  const sidebarButton = page.getByTestId('new-thread-sidebar-button');
  if (await sidebarButton.isVisible().catch(() => false)) {
    await sidebarButton.click({ force: true });
  } else {
    await page.getByTestId('new-thread-button').click({ force: true });
  }
  const changed = await expect
    .poll(
      async () => {
        const current = await selectedThreadId(page);
        return current && current !== before ? current : null;
      },
      { timeout: 10_000 }
    )
    .not.toBeNull()
    .then(
      () => true,
      () => false
    );
  const id = await selectedThreadId(page);
  if (changed && id) return id;
  if (id) return id;
  if (before) return before;
  throw new Error('selectedThreadId was not populated');
}

async function waitForSocketConnected(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => { socket?: { byUser?: Record<string, { status?: string }> } };
              };
            }
          ).__OPENHUMAN_STORE__;
          const byUser = store?.getState?.().socket?.byUser ?? {};
          return Object.values(byUser).some(entry => entry?.status === 'connected');
        }),
      { timeout: 30_000 }
    )
    .toBe(true);
}

async function sendMessage(page: Page, prompt: string): Promise<void> {
  await waitForSocketConnected(page);
  await dismissWalkthroughIfPresent(page);
  await page.getByPlaceholder('Type a message...').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

function hexEncode(value: string): string {
  return Buffer.from(value, 'utf8').toString('hex');
}

test.describe('Chat Harness - Wallet Flow', () => {
  test('sets up the wallet and drives the chat through the crypto tool path', async ({ page }) => {
    await resetMock();
    await bootAuthenticatedPage(page, USER_ID, '/home');
    await emulateTauriRuntime(page);
    await page.goto('/#/settings/recovery-phrase');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByRole('button', { name: 'Copy to Clipboard' })).toBeVisible();
    await page.locator('input[type="checkbox"]').first().check();
    await page.getByRole('button', { name: 'Save Recovery Phrase' }).click();

    await expect
      .poll(async () => {
        const wallet = await callCoreRpc<{
          result?: { configured?: boolean; accounts?: unknown[] };
        }>('openhuman.wallet_status', {});
        return {
          configured: Boolean(wallet.result?.configured),
          accountCount: wallet.result?.accounts?.length ?? 0,
        };
      })
      .toEqual({ configured: true, accountCount: expect.any(Number) });

    await setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, WALLET_PROMPT);

    await expect(
      page.getByText(
        /Prepared a wallet quote for John\..*wallet-quote-canary-8d13|Done\.\s*wallet-quote-canary-8d13/i
      )
    ).toBeVisible({ timeout: 40_000 });
  });
});
