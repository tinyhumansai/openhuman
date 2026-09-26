/**
 * Tool-call presentation, end to end.
 *
 * Drives a real core against the mock backend: the mock LLM calls the
 * managed web search and a file read, the core executes both, and the chat
 * renders them through assistant-ui's tool-timeline, tool-call and
 * web-search elements. Pins what the mislabelling bugs broke: the search reads
 * "Searched the web" (not a raw name), its hits render as the web-search
 * element, and a settled step reads in the past tense.
 */
import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-tool-call-presentation';

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
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

test.describe('Tool-call presentation', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await openChat(page);
    await createNewThread(page);
  });

  test('renders a web search and a file read as labelled timeline steps', async ({ page }) => {
    const CANARY = 'canary-tool-presentation-7f3e';
    const forced = [
      {
        content: '',
        toolCalls: [
          {
            id: 'call_web_search_1',
            name: 'web_search_tool',
            arguments: JSON.stringify({ query: 'rust async traits' }),
          },
          {
            id: 'call_file_read_1',
            name: 'file_read',
            // Read the E2E workspace config so the mock turn can complete;
            // a missing path is classified as an unsupported tool failure and
            // intentionally trips the harness circuit breaker before the
            // presentation assertions run.
            arguments: JSON.stringify({ path: 'tool-presentation-fixture.txt' }),
          },
        ],
      },
      { content: `Here is what I found. ${CANARY}` },
    ];
    await setMockBehavior('llmForcedResponses', JSON.stringify(forced));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'search the web for rust async traits and read the README');
    await expect(page.getByText(CANARY).last()).toBeVisible({ timeout: 60_000 });

    const timeline = page.locator('[data-slot="tool-group-root"]').last();
    await expect(timeline).toBeVisible();
    // The current chat surface renders the settled process trail through one
    // grouped disclosure. Open it before checking the individual labels.
    await timeline.locator('[data-slot="tool-group-trigger"]').click();
    await expect(timeline).toContainText('Searched the web');
    await expect(timeline).toContainText('Read file');
    await expect(page.getByText('file_read', { exact: true })).toHaveCount(0);
    await expect(page.getByText('web_search_tool', { exact: true })).toHaveCount(0);

    if (process.env.PW_TOOL_SCREENSHOT) {
      await timeline.screenshot({ path: process.env.PW_TOOL_SCREENSHOT });
    }
  });
});
