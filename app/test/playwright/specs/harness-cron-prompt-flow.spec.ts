import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-harness-cron-prompt-flow';

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
  await expect
    .poll(async () => {
      const current = await selectedThreadId(page);
      return current && current !== before ? current : null;
    })
    .not.toBeNull();
  const id = await selectedThreadId(page);
  if (!id) throw new Error('selectedThreadId was not populated');
  return id;
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

test.describe('Harness - Cron prompt-flow', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await openChat(page);
    await createNewThread(page);
  });

  test('natural-language create flow yields a final reply and may persist a job', async ({
    page,
  }) => {
    const CANARY = 'canary-cron-create-a1b2';
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_cron_add_1',
              name: 'cron_add',
              arguments: JSON.stringify({
                name: 'morning_reminder',
                schedule: '0 9 * * *',
                prompt: 'morning reminder',
                enabled: true,
              }),
            },
          ],
        },
        { content: `Done! I have set up a daily 9am morning reminder for you. ${CANARY}` },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'remind me every morning at 9am');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/Done! I have set up a daily 9am morning reminder/i)).toBeVisible();
    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
  });

  test('listing scheduled tasks returns the forced response', async ({ page }) => {
    const CANARY = 'canary-cron-list-c3d4';
    await setMockBehavior(
      'llmKeywordRules',
      JSON.stringify([
        {
          keyword: 'scheduled tasks',
          content: `You have 2 scheduled tasks: daily_standup (weekdays 9am) and weekly_review (Fridays 10am). ${CANARY}`,
        },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'what are my scheduled tasks');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/You have 2 scheduled tasks/i)).toBeVisible();
  });

  test('schedule update flow yields a final reply', async ({ page }) => {
    const CANARY = 'canary-cron-update-e5f6';
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_cron_update_1',
              name: 'cron_update',
              arguments: JSON.stringify({
                id: 'morning_reminder_update_test',
                schedule: '0 8 * * *',
              }),
            },
          ],
        },
        { content: `Done! I have changed your morning reminder to 8am. ${CANARY}` },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'change my morning reminder to 8am');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/changed your morning reminder to 8am/i)).toBeVisible();
  });

  test('delete flow yields a final reply', async ({ page }) => {
    const CANARY = 'canary-cron-delete-g7h8';
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_cron_remove_1',
              name: 'cron_remove',
              arguments: JSON.stringify({ id: 'morning_reminder_delete_test' }),
            },
          ],
        },
        { content: `Done! I have deleted the morning reminder. ${CANARY}` },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'delete the morning reminder');
    await expect(page.getByText(CANARY)).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/deleted the morning reminder/i)).toBeVisible();
  });
});
