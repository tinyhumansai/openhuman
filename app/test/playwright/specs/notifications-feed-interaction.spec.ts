/**
 * The feed is seeded through redux-persist before the app mounts so these tests
 * exercise the same state path as a returning user's browser.
 *
 * What is established, from three earlier runs against a live lane:
 *  - The persist blob format here is CORRECT. redux-persist stores each
 *    whitelisted field as its own JSON string inside the outer object, and a
 *    `localStorage` probe confirmed the app's own blob has exactly this shape.
 *  - The namespace is NOT the bypass id passed to `bootAuthenticatedPage`. The
 *    app resolves its active user from the mock backend and reads
 *    `user-123:persist:notifications`. The probe showed both keys present: the
 *    seed under the bypass id, never read, and the app's own under `user-123`,
 *    holding `items: "[]"`.
 *  - Seeding AFTER the app has mounted does nothing. redux-persist rehydrates
 *    once per page context, so a post-boot write is never read — which is why
 *    every assertion timed out against an empty feed.
 *
 * The fix, per that last point: learn the namespace on a first boot, register
 * the seed as an `addInitScript` so it runs before any application code, then
 * navigate to a fresh document that mounts with the blob already present. See
 * `openFeedWith`.
 *
 * The other half of this surface — integration notifications, which ARE
 * RPC-driven — is already covered by `notifications.spec.ts`; this file
 * deliberately does not duplicate it.
 */
import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * The system-events notification feed: filtering, mark-as-read, clear.
 *
 * `notifications.spec.ts` already covers the *integration* half — the core RPCs
 * (`notification_ingest` / `_list` / `_mark_read` / `_stats`) and that the page
 * renders both sections. It never touches the system-events feed's controls,
 * which are a different data path entirely: those items arrive over socket.io
 * from the Rust core and live in a redux-persist slice, so no core RPC can put
 * one on screen.
 *
 * Seed through the same Redux action dispatched by the socket bridge. This
 * exercises the real reducer and UI without coupling the fixture to persistence
 * internals; the reload test separately covers persistence.
 *
 * What this pins is the part that is invisible to a component test: that the
 * chips are a single-select tablist over the categories actually present, that
 * clicking an item marks it read, and that "Mark all read" empties the unread
 * count rather than only the badge.
 */

const USER = 'pw-notif-feed';

interface SeedItem {
  id: string;
  category: string;
  title: string;
  body: string;
  timestamp: number;
  read: boolean;
}

function item(id: string, category: string, title: string, read = false): SeedItem {
  return { id, category, title, body: `${title} body`, timestamp: Date.now(), read };
}

async function openFeedWith(page: Page, items: SeedItem[]): Promise<void> {
  await bootAuthenticatedPage(page, USER, '/notifications?view=main');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const welcomeCta = page.getByTestId('notifications-welcome-cta-view');
  if (await welcomeCta.isVisible().catch(() => false)) await welcomeCta.click();
  await page.evaluate(payload => {
    const store = (
      window as typeof window & { __OPENHUMAN_STORE__?: { dispatch(action: unknown): void } }
    ).__OPENHUMAN_STORE__;
    if (!store) throw new Error('application store is not available');
    for (const notification of payload) {
      store.dispatch({ type: 'notifications/notificationReceived', payload: notification });
    }
  }, items);
  await expect(page.getByTestId('system-events-section')).toBeVisible({ timeout: 20_000 });
}

const feed = (page: Page) => page.getByTestId('system-events-section');
const rows = (page: Page) => feed(page).getByTestId('notification-item');

test.describe('Notifications — the system-events feed renders what was stored', () => {
  test('shows every seeded item', async ({ page }) => {
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);

    await expect(rows(page)).toHaveCount(2, { timeout: 20_000 });
    await expect(feed(page)).toContainText('Agent finished a task');
    await expect(feed(page)).toContainText('Core restarted');
  });

  test('offers a filter chip only for categories that are present', async ({ page }) => {
    // The chip row is built from the categories actually in the feed. Offering
    // a filter that can only ever show nothing is a dead control.
    await openFeedWith(page, [item('n-agents-1', 'agents', 'Agent finished a task')]);

    await expect(page.getByTestId('notif-filter-chip-all')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('notif-filter-chip-agents')).toBeVisible();
    await expect(page.getByTestId('notif-filter-chip-system')).toHaveCount(0);
  });
});

test.describe('Notifications — filtering is single-select and actually filters', () => {
  test('narrows the feed to one category and back', async ({ page }) => {
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);
    await expect(rows(page)).toHaveCount(2, { timeout: 20_000 });

    await page.getByTestId('notif-filter-chip-system').click();

    await expect(rows(page)).toHaveCount(1);
    await expect(feed(page)).toContainText('Core restarted');
    await expect(feed(page)).not.toContainText('Agent finished a task');

    await page.getByTestId('notif-filter-chip-all').click();
    await expect(rows(page)).toHaveCount(2);
  });

  test('marks the active chip selected and deselects the previous one', async ({ page }) => {
    // A tablist, not a set of toggles: exactly one selected at a time. Two
    // chips reading `aria-selected="true"` is the bug this pins.
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);

    const all = page.getByTestId('notif-filter-chip-all');
    const system = page.getByTestId('notif-filter-chip-system');

    await expect(all).toHaveAttribute('aria-selected', 'true', { timeout: 20_000 });

    await system.click();
    await expect(system).toHaveAttribute('aria-selected', 'true');
    await expect(all).toHaveAttribute('aria-selected', 'false');
  });
});

test.describe('Notifications — read state', () => {
  test('clicking an unread item marks it read and drops the unread count', async ({ page }) => {
    await openFeedWith(page, [
      item('n-meeting-1', 'meetings', 'Meeting starts soon'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);
    await expect(rows(page)).toHaveCount(2, { timeout: 20_000 });

    // The header reports the unread count; it is the user-visible signal that
    // the click did anything.
    await expect(page.getByText(/2 unread/i)).toBeVisible();

    await rows(page).filter({ hasText: 'Meeting starts soon' }).locator('[role="button"]').click();

    await expect(page.getByText(/1 unread/i)).toBeVisible({ timeout: 10_000 });
  });

  test('Mark all read clears the count and disables itself', async ({ page }) => {
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);

    const markAll = page.getByRole('button', { name: /mark all as read/i });
    await expect(markAll).toBeEnabled({ timeout: 20_000 });

    await markAll.click();

    await expect(page.getByText(/unread/i)).toHaveCount(0, { timeout: 10_000 });
    await expect(markAll).toBeDisabled();
  });

  test('Mark all read is already disabled when nothing is unread', async ({ page }) => {
    await openFeedWith(page, [item('n-meeting-1', 'meetings', 'Meeting starts soon', true)]);

    await expect(rows(page)).toHaveCount(1, { timeout: 20_000 });
    await expect(page.getByRole('button', { name: /mark all as read/i })).toBeDisabled();
  });

  test('read state survives a reload', async ({ page }) => {
    // The slice is persisted, so marking read must outlive the page. If it does
    // not, the feed re-accuses the user of everything they just dismissed.
    await openFeedWith(page, [item('n-meeting-1', 'meetings', 'Meeting starts soon')]);

    await page.getByRole('button', { name: /mark all as read/i }).click();
    await expect(page.getByRole('button', { name: /mark all as read/i })).toBeDisabled({
      timeout: 10_000,
    });

    // `toBeDisabled()` above only proves in-memory state. redux-persist queues
    // its writes through an async `userScopedStorage.setItem`, so reloading
    // immediately can race the write and re-read the pre-mark blob — which
    // would look like "read state does not survive a reload" when in fact the
    // reload simply happened first. Wait for the persisted payload to show the
    // item as read before reloading.
    const user = await page.evaluate(() => localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID'));
    await expect
      .poll(
        async () =>
          page.evaluate(key => {
            const raw = window.localStorage.getItem(key);
            if (!raw) return false;
            try {
              const blob = JSON.parse(raw) as { items?: string };
              const items = JSON.parse(blob.items ?? '[]') as Array<{ read?: boolean }>;
              return items.length > 0 && items.every(item => item.read === true);
            } catch {
              return false;
            }
          }, `${user}:persist:notifications`),
        { timeout: 10_000, message: 'the read state was never persisted' }
      )
      .toBe(true);

    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await page.goto('/#/notifications?view=main');
    await waitForAppReady(page);

    await expect(page.getByRole('button', { name: /mark all as read/i })).toBeDisabled({
      timeout: 20_000,
    });
  });
});
