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

test.describe('Chat Tool Error Recovery', () => {
  test('surfaces an interrupted turn, clears in-flight state, and accepts a retry', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior(
      'llmStreamScript',
      JSON.stringify([{ text: 'Starting to answer', delayMs: 30 }, { error: 'upstream LLM error' }])
    );

    await openChat(page);
    const threadId = await createNewThread(page);
    await sendMessage(page, 'Tell me something important.');

    await expect(page.getByText('Starting to answer')).toBeVisible({ timeout: 20_000 });

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

    const composer = page.getByPlaceholder('Type a message...');
    await expect(composer).toBeEnabled();

    await setMockBehavior('llmStreamScript', '');
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([{ content: `Recovery successful: ${RECOVERY_CANARY}` }])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'Please try again with a fresh answer.');
    await expect(page.getByText(RECOVERY_CANARY)).toBeVisible({ timeout: 30_000 });
  });
});
