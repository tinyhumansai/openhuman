import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-error-recovery';
const RECOVERY_CANARY = 'canary-recovery-7g8h9i';

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
  await expect(page.getByTestId('chat-message-input')).toBeVisible();
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
  await page.getByTestId('chat-message-input').fill(prompt);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

test.describe('Chat Tool Error Recovery', () => {
  test('surfaces an interrupted turn, clears in-flight state, and accepts a retry', async ({
    page,
  }) => {
    await resetMock();
    // Route both turns up front. Changing the global stream script between
    // an error and its retry leaves a shared mock-server race when this spec
    // follows another fault-injection case in the same browser session.
    await setMockBehavior(
      'llmKeywordRules',
      JSON.stringify([
        {
          keyword: 'Please try again with a fresh answer.',
          streamScript: [{ text: `Recovery successful: ${RECOVERY_CANARY}` }, { finish: 'stop' }],
        },
        {
          keyword: 'Tell me something important.',
          streamScript: [{ text: 'Starting to answer', delayMs: 30 }, { error: 'invalid API key' }],
        },
      ])
    );

    await openChat(page);
    const threadId = await createNewThread(page);
    await sendMessage(page, 'Tell me something important.');

    // A new thread starts with a null lifecycle too. Waiting only for null can
    // therefore race ahead of the first turn and let the retry overlap it.
    // The error card proves the failed turn actually ran before we retry.
    await expect(page.getByText('Something went wrong')).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(async () => {
        const lifecycle = await page.evaluate(currentThreadId => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => {
                  chatRuntime?: { inferenceTurnLifecycleByThread?: Record<string, string | null> };
                };
              };
            }
          ).__OPENHUMAN_STORE__;
          return (
            store?.getState?.().chatRuntime?.inferenceTurnLifecycleByThread?.[currentThreadId] ??
            null
          );
        }, threadId);
        return lifecycle;
      })
      .toBeNull();

    const composer = page.getByTestId('chat-message-input');
    await expect(composer).toBeEnabled();

    await sendMessage(page, 'Please try again with a fresh answer.');
    await expect(page.getByText(RECOVERY_CANARY)).toBeVisible({ timeout: 30_000 });
  });
});
