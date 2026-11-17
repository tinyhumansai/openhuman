import { expect, type Page, test } from '@playwright/test';

import { agentMessageText } from '../helpers/chat-locators';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-tool-call';
const PROMPT = 'Fetch the contents of https://example.com for me.';
const CANARY_FINAL = 'canary-tool-call-fetched-a1b2c3';
const CANARY_SECOND_PARAGRAPH = 'The entire answer must stay in this same assistant bubble.';
const FINAL_RESPONSE = `Here is the fetched content: ${CANARY_FINAL}\n\n${CANARY_SECOND_PARAGRAPH}`;
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
  { content: FINAL_RESPONSE },
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

test.describe('Chat Tool Call Flow', () => {
  test('renders a terminal tool call, final answer, and clears in-flight state', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    const threadId = await createNewThread(page);
    await sendMessage(page, PROMPT);

    await expect(agentMessageText(page, CANARY_FINAL)).toBeVisible({ timeout: 40_000 });
    const finalBubble = page.getByTestId('agent-message').filter({ hasText: CANARY_FINAL });
    await expect(finalBubble).toHaveCount(1);
    await expect(finalBubble).toContainText(CANARY_SECOND_PARAGRAPH);
    await expect(page.getByText(CANARY_SECOND_PARAGRAPH, { exact: true })).toHaveCount(1);

    // Regression: completed tool/reasoning arrays remain in Redux briefly, but
    // they must not create a synthetic running tail after the final answer.
    await expect(page.getByLabel('Assistant is working')).toHaveCount(0);
    await expect(page.getByText('running', { exact: true })).toHaveCount(0);

    // Tool activity belongs to assistant-ui and renders as a readable card,
    // never through the removed legacy Agentic task insights timeline.
    await expect(page.getByTestId('agent-task-insights')).toHaveCount(0);
    const activityGroup = await openLatestActivityGroup(page);
    const toolCard = activityGroup.locator(
      '[data-testid="assistant-ui-tool-call"][data-tool-name="web_fetch"]'
    );
    await expect(toolCard).toHaveCount(1);
    await expect(toolCard).toBeVisible();
    await expect(toolCard).toContainText('Read webpage');
    await expect(toolCard).toContainText('example.com');
    // The mock fetch completes successfully in this fixture. Assert the real
    // terminal result, rather than assuming a cancellation that this flow does
    // not exercise.
    await expect(toolCard).toHaveAttribute('data-outcome', 'success');
    await expect(toolCard.getByTestId('tool-call-status')).toHaveText('done');
    await expect(toolCard).not.toContainText('running');
    const toolTrigger = toolCard.getByRole('button').first();
    if ((await toolTrigger.getAttribute('aria-expanded')) !== 'true') await toolTrigger.click();
    await expect(toolTrigger).toHaveAttribute('aria-expanded', 'true');
    await expect(toolCard.getByTestId('tool-body-fetch')).toBeVisible();

    expect(threadId.length).toBeGreaterThan(0);
  });
});
