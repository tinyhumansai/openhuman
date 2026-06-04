import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-scroll-render';
const CANARY_BOLD = 'BOLD-CANARY-22ff';
const CANARY_CODE = 'CODE-CANARY-93b1';
const LINK_URL = 'https://example.com/canary';
const REPLY_MARKDOWN = [
  `**${CANARY_BOLD}** is bold.`,
  '',
  '```',
  `${CANARY_CODE}`,
  'line 2',
  '```',
  '',
  `Visit [the docs](${LINK_URL}) for more.`,
].join('\n');
const FILLER_LINES = Array.from({ length: 30 }, (_, index) => `Filler line ${index + 1}.`);
const STREAM_SCRIPT = [
  ...FILLER_LINES.map(line => ({ text: `${line}\n`, delayMs: 5 })),
  { text: '\n', delayMs: 5 },
  { text: REPLY_MARKDOWN, delayMs: 10 },
  { finish: 'stop' },
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

async function createNewThread(page: Page): Promise<void> {
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
  if (!changed && !id && !before) {
    throw new Error('selectedThreadId was not populated');
  }
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

test.describe('Chat Harness - Scroll Render', () => {
  test('renders markdown and releases bottom-stick when the user scrolls up', async ({ page }) => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(STREAM_SCRIPT));
    await setMockBehavior('llmStreamChunkDelayMs', '5');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, 'Reply with the markdown sample please.');

    await expect(page.getByText(CANARY_BOLD)).toBeVisible({ timeout: 40_000 });
    await expect(page.getByText(CANARY_CODE)).toBeVisible({ timeout: 20_000 });

    const tags = await page.evaluate(() => {
      const column = document.querySelector(
        'div.flex-1.overflow-y-auto.bg-\\[\\#f6f6f6\\]'
      ) as HTMLElement | null;
      return {
        scrollTop: column?.scrollTop ?? 0,
        scrollHeight: column?.scrollHeight ?? 0,
        clientHeight: column?.clientHeight ?? 0,
      };
    });

    await expect(page.getByText(CANARY_BOLD)).toBeVisible();
    await expect(page.getByText(CANARY_CODE)).toBeVisible();
    await expect(page.getByText('the docs')).toBeVisible();
    expect(tags.scrollHeight).toBeGreaterThanOrEqual(tags.clientHeight);
    if (tags.scrollHeight > tags.clientHeight) {
      const initialRemaining = tags.scrollHeight - (tags.scrollTop + tags.clientHeight);
      expect(initialRemaining).toBeLessThan(40);

      const targetTop = Math.max(0, tags.scrollTop - Math.floor(tags.clientHeight / 2));
      await page.evaluate(nextTop => {
        const column = document.querySelector(
          'div.flex-1.overflow-y-auto.bg-\\[\\#f6f6f6\\]'
        ) as HTMLElement | null;
        column?.scrollTo({ top: nextTop, behavior: 'auto' });
      }, targetTop);

      await page.waitForTimeout(500);

      const afterScrollUp = await page.evaluate(() => {
        const column = document.querySelector(
          'div.flex-1.overflow-y-auto.bg-\\[\\#f6f6f6\\]'
        ) as HTMLElement | null;
        return {
          scrollTop: column?.scrollTop ?? 0,
          scrollHeight: column?.scrollHeight ?? 0,
          clientHeight: column?.clientHeight ?? 0,
        };
      });

      expect(Math.abs(afterScrollUp.scrollTop - targetTop)).toBeLessThan(40);
      expect(afterScrollUp.scrollTop).toBeLessThan(tags.scrollTop - 20);
      expect(
        afterScrollUp.scrollHeight - (afterScrollUp.scrollTop + afterScrollUp.clientHeight)
      ).toBeGreaterThan(initialRemaining + 10);
    }
  });
});
