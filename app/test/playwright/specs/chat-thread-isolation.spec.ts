/**
 * Thread isolation — a turn streaming on one conversation must stay attributed
 * to it while the user reads or starts another.
 *
 * The app models this per-thread: `activeThreadIds`, `processingByThread` and
 * `queuedFollowupsByThread` are all maps keyed by thread id
 * (`Conversations.tsx:271-282`), and `selectedThreadActive` is
 * `selectedThreadId && activeThreadIds[selectedThreadId]`. Nothing in the
 * browser suite exercised the switch, though: thread coverage was one
 * history-persistence case (`chat-conversation-history`) and one rename/delete
 * case (`chat-management-functional`).
 *
 * The failure this guards against is bleed — the new thread showing the old
 * thread's tokens, or offering a Stop button that would cancel a turn the user
 * is no longer looking at. Both are silent: nothing errors, the transcript just
 * shows the wrong conversation.
 *
 * Behaviour was observed on the running app before these assertions were
 * written, not inferred from the source. Recorded in
 * `~/tinyhuman/bugs/W2-ui-bugs.md` as OBS-13.
 *
 * NOT asserted here, deliberately: the composer draft carries across a switch
 * (`inputValue` is one global `useState`, not a per-thread map). That is
 * BUG-W2-UI-2, a product decision rather than a broken contract — pinning it
 * would entrench behaviour that may be wrong, and whoever fixes it would have
 * to delete the test.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-thread-isolation';

/** `safeDelayMs` clamps to 1000ms, so length comes from chunk count. ~24s. */
const SLOW_STREAM = [
  ...Array.from({ length: 24 }, (_, i) => ({ text: `tokenA${i} `, delayMs: 1000 })),
  { finish: 'stop' },
];

const A_PROMPT = 'ALPHA-THREAD-PROMPT';

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
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

const composer = (page: Page): Locator => page.getByTestId('chat-message-input');
const stopButton = (page: Page): Locator => page.getByTestId('stop-generation-button');

/** The selected thread id, read from the live store. */
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

async function startNewThread(page: Page): Promise<void> {
  await dismissWalkthroughIfPresent(page);
  const sidebar = page.getByTestId('new-thread-sidebar-button');
  if (await sidebar.isVisible().catch(() => false)) {
    await sidebar.click({ force: true });
  } else {
    await page.getByTestId('new-thread-button').click({ force: true });
  }
}

/** Send on the current thread and leave the turn streaming. */
async function streamOnCurrentThread(page: Page, prompt: string): Promise<string> {
  await composer(page).click();
  await page.keyboard.type(prompt);
  await page.keyboard.press('Enter');
  await expect(stopButton(page)).toBeVisible({ timeout: 20_000 });
  const id = await selectedThreadId(page);
  expect(id, 'a thread id must exist once a turn is in flight').not.toBeNull();
  return id as string;
}

/**
 * These are browser specs against a freshly built bundle, and the first few to
 * run pay the app's cold start: a fresh `dist-web` plus a just-rebuilt core
 * means first paint can take most of a minute, while every subsequent test in
 * the same session settles at ~1s.
 *
 * Measured on this suite: cases 1-4 of the first spec failed at ~60s with a
 * blank `#root`, case 5 of the SAME file passed at 25.3s, and all 13 cases
 * after it passed in ~1s. Nothing about the app was wrong — the per-test budget
 * (60s locally, `playwright.config.ts:10`) was simply consumed by warm-up.
 *
 * Raising the budget for this describe rather than editing the shared config:
 * it is a statement about these tests, it masks nothing (the assertions are
 * unchanged and a genuinely broken app still fails), and whichever spec happens
 * to sort first should not be the one that flakes.
 */
test.describe.configure({ timeout: 120_000 });

test.describe('Chat thread isolation during a stream', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(SLOW_STREAM));
    await setMockBehavior('llmStreamChunkDelayMs', '1000');
  });

  /*
   * REMOVED — 'a turn keeps running on its own thread after the user starts
   * another'.
   *
   * It asserted `activeThreadIds` from the Redux store, which is an internals
   * assertion, not something a user can see — and it is redundant: the
   * "switching back" case below proves the same property through the UI, by
   * showing that A's Stop control is still live when you return to it. A turn
   * that had been cancelled by navigation could not do that.
   *
   * Dropped rather than kept as a cheap extra green: two assertions of one
   * property, one of them reaching past the UI, is not more coverage.
   */

  test('the new thread offers no Stop control for the other thread’s turn', async ({ page }) => {
    // The bleed that matters most: a Stop button on B would cancel A's turn,
    // which the user is no longer looking at and did not ask to stop.
    await openChat(page);
    const threadA = await streamOnCurrentThread(page, A_PROMPT);

    await startNewThread(page);
    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadA);

    await expect(
      stopButton(page),
      'the in-flight turn belongs to the other thread and must not be stoppable from here'
    ).toHaveCount(0);
  });

  test('the new thread’s transcript shows none of the streaming thread’s content', async ({
    page,
  }) => {
    await openChat(page);
    const threadA = await streamOnCurrentThread(page, A_PROMPT);
    // Wait for real streamed tokens, so this is not merely a race with an empty
    // transcript — there is something that COULD bleed by the time we switch.
    await expect(page.getByText('tokenA0', { exact: false }).last()).toBeVisible({
      timeout: 20_000,
    });

    await startNewThread(page);
    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadA);

    await expect(page.getByText(A_PROMPT, { exact: false })).toHaveCount(0);
    await expect(page.getByText('tokenA0', { exact: false })).toHaveCount(0);
  });

  test('switching back restores the original thread and its in-flight turn', async ({ page }) => {
    // The other half: isolation must not mean the turn is orphaned. Coming
    // back has to show A's transcript again, with its Stop control live.
    await openChat(page);
    const threadA = await streamOnCurrentThread(page, A_PROMPT);

    await startNewThread(page);
    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadA);
    // NOT asserting a row count. `bootAuthenticatedPage` resets the session but
    // not the thread list, so threads accumulate across the cases in this file
    // for the same user id — an earlier draft expected 2 and found 5, which is
    // a fact about test ordering rather than about isolation. What matters is
    // that A's own row is there and selects A.
    const rowA = page.getByTestId(`thread-row-${threadA}`);
    await expect(rowA).toBeVisible({ timeout: 15_000 });
    await rowA.click({ force: true });

    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).toBe(threadA);
    await expect(page.getByText(A_PROMPT, { exact: false }).last()).toBeVisible({
      timeout: 15_000,
    });
    await expect(
      stopButton(page),
      'returning to a streaming thread must show its Stop control again'
    ).toBeVisible({ timeout: 15_000 });
  });
});
