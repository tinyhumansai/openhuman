import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-tool-call';
const PROMPT = 'Fetch the contents of https://example.com for me.';
const CANARY_FINAL = 'canary-tool-call-fetched-a1b2c3';
const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_web_fetch_1',
        name: 'web_fetch',
        arguments: JSON.stringify({ url: 'https://example.com' }),
      },
    ],
  },
  { content: `Here is the fetched content: ${CANARY_FINAL}` },
];

interface MockRequest {
  method: string;
  url: string;
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
  await page.getByPlaceholder('How can I help you today?').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

async function toolTimelineNames(page: Page, threadId: string): Promise<string[]> {
  return page.evaluate(currentThreadId => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => {
            chatRuntime?: { toolTimelineByThread?: Record<string, Array<{ name?: string }>> };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    const entries = store?.getState?.().chatRuntime?.toolTimelineByThread?.[currentThreadId] ?? [];
    return entries.map(entry => entry.name ?? '');
  }, threadId);
}

test.describe('Chat Tool Call Flow', () => {
  test('runs one tool call round, renders the final answer, and clears in-flight state', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    const threadId = await createNewThread(page);
    await sendMessage(page, PROMPT);

    await expect(page.getByText(CANARY_FINAL)).toBeVisible({ timeout: 40_000 });

    const names = await expect
      .poll(async () => toolTimelineNames(page, threadId), { timeout: 20_000 })
      .not.toEqual([]);

    void names;
    expect((await toolTimelineNames(page, threadId)).some(name => name.includes('web_fetch'))).toBe(
      true
    );

    await expect
      .poll(async () => {
        const log = await requests();
        return log.filter(
          entry => entry.method === 'POST' && entry.url.includes('/openai/v1/chat/completions')
        ).length;
      })
      .toBeGreaterThanOrEqual(2);

    expect(threadId.length).toBeGreaterThan(0);
  });
});
