import { expect, type Page, test } from '@playwright/test';

import { agentMessageText } from '../helpers/chat-locators';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-multi-tool';
const PROMPT = 'Read the config file and search for the relevant setting.';
const CANARY_FINAL = 'canary-multi-tool-d4e5f6';
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
  {
    content: '',
    toolCalls: [
      {
        id: 'call_web_search_1',
        name: 'web_search_tool',
        arguments: JSON.stringify({ query: 'openhuman relevant setting' }),
      },
    ],
  },
  { content: `Found the content using both tools: ${CANARY_FINAL}` },
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

async function openLatestActivityGroup(page: Page) {
  const group = page.locator('[data-slot="tool-group-root"]').last();
  await expect(group).toBeVisible({ timeout: 30_000 });
  const trigger = group.locator('[data-slot="tool-group-trigger"]');
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  return group;
}

test.describe('Chat Multi Tool Round', () => {
  test('renders web_fetch then web_search_tool cards before the final answer', async ({ page }) => {
    await resetMock();
    await setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, PROMPT);

    await expect(agentMessageText(page, CANARY_FINAL)).toBeVisible({ timeout: 50_000 });
    const activityGroup = await openLatestActivityGroup(page);
    const cards = activityGroup.getByTestId('assistant-ui-tool-call');
    await expect(cards).toHaveCount(2);
    await expect(cards.nth(0)).toHaveAttribute('data-tool-name', 'web_fetch');
    await expect(cards.nth(1)).toHaveAttribute('data-tool-name', 'web_search_tool');
    await expect(cards.nth(0)).toContainText('Read webpage');
    await expect(cards.nth(0)).toContainText('example.com');
    await expect(cards.nth(1)).toContainText(/Search(?:ing|ed) the web/);
    await expect(agentMessageText(page, CANARY_FINAL)).toBeVisible();
  });
});
