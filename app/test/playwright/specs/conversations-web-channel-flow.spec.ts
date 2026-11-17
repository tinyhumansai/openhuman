import { expect, type Page, test } from '@playwright/test';

import { inFlightThreadKeys } from '../helpers/chat-transcript';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-conversations-web-channel';
const PROMPT = 'hello from playwright web channel';
const REPLY = 'Hello from e2e mock agent';

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

test.describe('Conversations web channel flow', () => {
  test('sends a UI message through the agent loop and renders the response', async ({ page }) => {
    await resetMock();
    const script = [{ text: REPLY }, { finish: 'stop' }];
    await setMockBehavior('llmStreamScript', JSON.stringify(script));

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, PROMPT);

    // assistant-ui renders user text in its message root rather than a
    // paragraph. Keep this scoped to the user bubble so a matching string in
    // a transcript/tool card cannot satisfy the assertion.
    await expect(
      page.locator('[data-slot="aui_user-message-root"]').filter({ hasText: PROMPT }).first()
    ).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('agent-message').filter({ hasText: REPLY }).first()).toBeVisible({
      timeout: 30_000,
    });

    await expect
      .poll(async () => {
        const log = await requests();
        return log.some(
          entry => entry.method === 'POST' && entry.url.includes('/openai/v1/chat/completions')
        );
      })
      .toBe(true);
  });

  test('continues an in-flight response after navigating away and back', async ({ page }) => {
    await resetMock();
    const reply = 'The response continued while the chat tab was away.';
    await setMockBehavior(
      'llmStreamScript',
      JSON.stringify([{ text: reply, delayMs: 1_500 }, { finish: 'stop' }])
    );

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, 'continue this response after a tab switch');
    await expect(page.getByTestId('stop-generation-button')).toBeVisible({ timeout: 10_000 });

    await page.evaluate(() => {
      window.location.hash = '/connections';
    });
    await expect.poll(() => page.evaluate(() => window.location.hash)).toContain('#/connections');
    await expect.poll(inFlightThreadKeys).not.toEqual([]);

    await page.evaluate(() => {
      window.location.hash = '/chat';
    });
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
    await expect(page.getByTestId('agent-message').filter({ hasText: reply })).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.locator('#root')).not.toContainText('Something went wrong');
    await expect.poll(inFlightThreadKeys).toEqual([]);
  });
});
