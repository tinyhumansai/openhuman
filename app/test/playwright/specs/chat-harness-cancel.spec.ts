import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-cancel';
const PROMPT = 'Please count to ten slowly with one number per chunk.';
const LATE_PIECES = ['five ', 'six.'];
const STREAM_SCRIPT = [
  { text: 'one ', delayMs: 500 },
  { text: 'two ', delayMs: 500 },
  { text: 'three ', delayMs: 500 },
  { text: 'four ', delayMs: 500 },
  { text: 'five ', delayMs: 500 },
  { text: 'six.', delayMs: 500 },
  { finish: 'stop' },
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

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
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
  await page.getByPlaceholder('How can I help you today?').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

test.describe('Chat Harness - Cancel', () => {
  test('cancels a mid-stream turn and leaves the composer interactive', async ({ page }) => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(STREAM_SCRIPT));
    await setMockBehavior('llmStreamChunkDelayMs', '500');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, PROMPT);

    // Mid-stream, the composer's Send button morphs into a Stop button (the
    // cancel control now lives in the composer for the text composer).
    await expect(page.getByTestId('stop-generation-button')).toBeVisible({ timeout: 10_000 });

    await page.getByTestId('stop-generation-button').click();

    await expect(page.getByTestId('stop-generation-button')).toHaveCount(0, { timeout: 10_000 });
    for (const piece of LATE_PIECES) {
      await expect(page.getByText(piece, { exact: false })).toHaveCount(0, { timeout: 5_000 });
    }

    const composer = page.getByPlaceholder('How can I help you today?');
    await expect(composer).toBeEnabled();
    await composer.fill('post-cancel probe message');
    await expect(page.getByTestId('send-message-button')).toBeEnabled();
  });
});
