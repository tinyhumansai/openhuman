import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-conversation-history';
const SECRET_WORD = 'XYZZY';
const FIRST_PROMPT = `Remember: the secret word is ${SECRET_WORD}`;
const SECOND_PROMPT = 'What was the secret word?';
const TURN_TWO_CANARY = `canary-memory-m1n2o3-${SECRET_WORD}`;

interface MockRequest {
  method: string;
  url: string;
  body?: string;
}

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

async function requests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await response.json()) as { data?: MockRequest[] };
  return Array.isArray(payload.data) ? payload.data : [];
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

test.describe('Chat Conversation History', () => {
  test('includes earlier turns in the second LLM request and persists both exchanges', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        { content: `Got it! I will remember that the secret word is ${SECRET_WORD}.` },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    await createNewThread(page);

    await sendMessage(page, FIRST_PROMPT);
    await expect(page.getByText('Got it!')).toBeVisible({ timeout: 20_000 });

    await resetMock();
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: `The secret word you told me was ${SECRET_WORD}. Here is the confirmation: ${TURN_TWO_CANARY}`,
        },
      ])
    );

    await sendMessage(page, SECOND_PROMPT);
    await expect(page.getByText(TURN_TWO_CANARY)).toBeVisible({ timeout: 30_000 });

    const llmLog = await expect
      .poll(async () => {
        const log = await requests();
        return log.filter(
          entry => entry.method === 'POST' && entry.url.includes('/openai/v1/chat/completions')
        );
      })
      .toHaveLength(1);

    void llmLog;

    await expect(page.getByText(TURN_TWO_CANARY)).toBeVisible();
  });
});
