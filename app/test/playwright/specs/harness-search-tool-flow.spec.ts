import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-harness-search-tool-flow';

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
  await page.getByPlaceholder('How can I help you today?').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

function findToolInLlmLog(log: MockRequest[], toolName: string): boolean {
  return log.some(
    request =>
      request.method === 'POST' &&
      request.url.includes('/chat/completions') &&
      typeof request.body === 'string' &&
      request.body.includes(`"${toolName}"`)
  );
}

test.describe('Harness - Search tool-flow', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await openChat(page);
    await createNewThread(page);
  });

  test('memory_recall prompt completes the two-turn sequence', async ({ page }) => {
    const CANARY = 'canary-memory-recall-a1b2';
    const forced = [
      {
        content: '',
        toolCalls: [
          {
            id: 'call_memory_recall_1',
            name: 'memory_recall',
            arguments: JSON.stringify({ query: 'project Atlas' }),
          },
        ],
      },
      {
        content: `Based on my memory search, we discussed project Atlas in relation to the Q4 infrastructure migration. ${CANARY}`,
      },
    ];
    await setMockBehavior('llmForcedResponses', JSON.stringify(forced));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'what did we discuss about project Atlas');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/Based on my memory search/i)).toBeVisible();

    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
    expect(findToolInLlmLog(log, 'memory_recall')).toBe(true);
  });

  test('web_search_tool prompt completes the two-turn sequence', async ({ page }) => {
    const CANARY = 'canary-web-search-c3d4';
    const forced = [
      {
        content: '',
        toolCalls: [
          {
            id: 'call_web_search_1',
            name: 'web_search_tool',
            arguments: JSON.stringify({ query: 'Rust async best practices' }),
          },
        ],
      },
      {
        content: `Here are the top results for Rust async best practices: use tokio for runtimes, prefer async/await over manual Future impls. ${CANARY}`,
      },
    ];
    await setMockBehavior('llmForcedResponses', JSON.stringify(forced));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'search for Rust async best practices');
    await expect(page.getByText(CANARY).first()).toBeVisible({ timeout: 60_000 });
    await expect(
      page.getByText(/Here are the top results for Rust async best practices/i)
    ).toBeVisible();

    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
    expect(findToolInLlmLog(log, 'web_search_tool')).toBe(true);
  });

  test('file_read prompt completes the two-turn sequence', async ({ page }) => {
    const CANARY = 'canary-file-read-e5f6';
    const FILE_SNIPPET = 'OpenHuman is an AI assistant for communities';
    const forced = [
      {
        content: '',
        toolCalls: [
          {
            id: 'call_file_read_1',
            name: 'file_read',
            arguments: JSON.stringify({ path: '/workspace/README.md' }),
          },
        ],
      },
      { content: `The README says: ${FILE_SNIPPET}. ${CANARY}` },
    ];
    await setMockBehavior('llmForcedResponses', JSON.stringify(forced));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'read the README');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/OpenHuman is an AI assistant/i)).toBeVisible();

    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
    expect(findToolInLlmLog(log, 'file_read')).toBe(true);
  });
});
