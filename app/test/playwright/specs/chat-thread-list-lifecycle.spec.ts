/**
 * Thread-list lifecycle and the `/chat/:threadId` deep link.
 *
 * # Why this is not covered elsewhere
 *
 * Of the ~16 chat specs, only `conversations-web-channel-flow` touches thread
 * switching at all, and it does so incidentally (it asserts an in-flight turn
 * survives a tab switch). Nothing covers the sidebar's own CRUD — create,
 * rename, delete — or the deep link, even though `/chat/:threadId?` is a
 * declared route (`AppRoutes.tsx:175`) and the URL users actually share.
 *
 * The deep link is the interesting one. `Conversations.tsx:670-685` resolves
 * `routeThreadId` through a branch that **deliberately bypasses the General-tab
 * visibility filter**, so that a task-labelled thread can be opened by URL even
 * though the sidebar would never list it. That branch has no coverage, and the
 * filter it bypasses is live (`selectedLabel` is hardcoded to General at
 * `:311`), so a regression there silently makes shared links land on the wrong
 * thread — or on a fresh empty one, which looks like data loss.
 *
 * # Locators
 *
 * The row action buttons carry `data-analytics-id` rather than a testid, and
 * are `hidden` until `group-hover` (`ThreadList.tsx:236`, `:247`), so each is
 * reached by hovering the row first. Rows are `thread-row-<id>`, the rename
 * field is `thread-title-input-<id>`, and delete goes through an AlertDialog
 * whose confirm button is labelled `common.delete` ("Delete").
 */
import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-thread-lifecycle';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
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
  await page.getByTestId('new-thread-button').click({ force: true });
  await expect
    .poll(
      async () => {
        const current = await selectedThreadId(page);
        return current && current !== before ? current : null;
      },
      { timeout: 15_000 }
    )
    .not.toBeNull();
  const id = await selectedThreadId(page);
  if (!id) throw new Error('selectedThreadId was not populated after create');
  return id;
}

/** Reveal a row's hover-only actions and return the row locator. */
async function hoverRow(page: Page, threadId: string) {
  const row = page.getByTestId(`thread-row-${threadId}`);
  await expect(row).toBeVisible();
  await row.hover();
  return row;
}

test.describe('Chat thread list — create, deep link, rename, delete', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  test('the sidebar creates a thread and selects it', async ({ page }) => {
    await openChat(page);

    const first = await createNewThread(page);
    const second = await createNewThread(page);

    // Two distinct threads, and the newest is the selected one. Asserting
    // inequality is what stops this passing if create silently no-ops and
    // re-selects the existing thread.
    expect(second).not.toBe(first);
    await expect(page.getByTestId(`thread-row-${first}`)).toBeVisible();
    await expect(page.getByTestId(`thread-row-${second}`)).toBeVisible();
    expect(await selectedThreadId(page)).toBe(second);
  });

  test('a /chat/:threadId deep link selects that thread, not the most recent one', async ({
    page,
  }) => {
    await openChat(page);

    const target = await createNewThread(page);
    const decoy = await createNewThread(page);
    expect(decoy).not.toBe(target);

    // `decoy` is newest and is what a plain `/chat` visit would resume, so
    // landing on `target` can only come from the route branch.
    await page.goto(`/#/chat/${target}`);
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect.poll(() => selectedThreadId(page), { timeout: 15_000 }).toBe(target);
  });

  test('an unknown thread id in the URL falls back to /chat instead of hanging', async ({
    page,
  }) => {
    await openChat(page);
    await createNewThread(page);

    // `Conversations.tsx:684-686` navigates back to `/chat` when the requested
    // thread is not in the list. Without that branch the surface would sit on a
    // route pointing at nothing.
    await page.goto('/#/chat/thread-that-does-not-exist');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect.poll(() => page.url(), { timeout: 15_000 }).toMatch(/#\/chat\/?$/);
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
  });

  test('renaming a thread from the sidebar persists the new title', async ({ page }) => {
    await openChat(page);
    const threadId = await createNewThread(page);

    await hoverRow(page, threadId);
    await page.locator('[data-analytics-id="chat-sidebar-edit-thread-title"]').first().click();

    const input = page.getByTestId(`thread-title-input-${threadId}`);
    await expect(input).toBeVisible();
    const RENAMED = 'renamed-thread-9q4w2e';
    await input.fill(RENAMED);
    await input.press('Enter');

    // The edit field closes and the row shows the new title.
    await expect(input).toHaveCount(0);
    await expect(page.getByTestId(`thread-row-${threadId}`)).toContainText(RENAMED);

    // And it survives a reload — proves the rename was committed to the
    // backend, not just held in local component state.
    await page.goto('/#/chat');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await expect(page.getByTestId(`thread-row-${threadId}`)).toContainText(RENAMED);
  });

  test('Escape cancels a rename without committing it', async ({ page }) => {
    await openChat(page);
    const threadId = await createNewThread(page);

    const titleBefore = (await page.getByTestId(`thread-row-${threadId}`).innerText()).trim();

    await hoverRow(page, threadId);
    await page.locator('[data-analytics-id="chat-sidebar-edit-thread-title"]').first().click();

    const input = page.getByTestId(`thread-title-input-${threadId}`);
    await expect(input).toBeVisible();
    await input.fill('this-must-not-stick-5t8y1u');

    // Escape is an explicit cancel and must suppress the commit the ensuing
    // blur would otherwise fire (`ThreadList.tsx:183-187`).
    await input.press('Escape');

    await expect(input).toHaveCount(0);
    await expect(page.getByTestId(`thread-row-${threadId}`)).not.toContainText(
      'this-must-not-stick-5t8y1u'
    );
    await expect(page.getByTestId(`thread-row-${threadId}`)).toContainText(titleBefore);
  });

  test('deleting a thread asks for confirmation and removes the row', async ({ page }) => {
    await openChat(page);
    const keep = await createNewThread(page);
    const doomed = await createNewThread(page);

    await hoverRow(page, doomed);
    await page.locator('[data-analytics-id="chat-sidebar-delete-thread"]').first().click();

    // Confirmation is required — the row must still be there while the dialog
    // is open, or the "are you sure" is decorative.
    const dialog = page.getByRole('alertdialog');
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId(`thread-row-${doomed}`)).toBeVisible();

    await dialog.getByRole('button', { name: 'Delete' }).click();

    await expect(page.getByTestId(`thread-row-${doomed}`)).toHaveCount(0, { timeout: 15_000 });
    // The untouched thread is still there — proves the delete was scoped.
    await expect(page.getByTestId(`thread-row-${keep}`)).toBeVisible();
  });

  test('cancelling the delete dialog keeps the thread', async ({ page }) => {
    await openChat(page);
    const threadId = await createNewThread(page);

    await hoverRow(page, threadId);
    await page.locator('[data-analytics-id="chat-sidebar-delete-thread"]').first().click();

    const dialog = page.getByRole('alertdialog');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel' }).click();

    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId(`thread-row-${threadId}`)).toBeVisible();
  });
});
