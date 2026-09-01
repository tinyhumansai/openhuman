/**
 * ⚠️ STILL SKIPPED — the seeding was restructured but has NOT been executed.
 *
 * Local test execution is currently forbidden by a standing fleet rule, so the
 * fix below is reasoned from source and verified by reading only. It is left
 * skipped rather than un-skipped-and-hoped: shipping an unrun spec as active is
 * how a red lane gets blamed on the wrong change. Un-skip on the first run that
 * can actually execute it.
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
 * Seeding therefore goes through the app's own persistence, not a mock: the
 * slice is persisted under `${activeUserId}:persist:notifications`
 * (`store/index.ts:144-149`, `store/userScopedStorage.ts:177-180`), and
 * `OPENHUMAN_ACTIVE_USER_ID` selects the namespace. Writing those two keys
 * before navigation is exactly what a returning user's browser already holds —
 * the real reducer rehydrates them, the real page renders them, and every
 * assertion below is on what the user can see and click.
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

/**
 * Learn the namespace the app actually persists under.
 *
 * `activeUserId` is the signed-in identity the app resolves, NOT the bypass id
 * handed to `bootAuthenticatedPage` — in this lane the mock backend answers
 * `user-123`. Seeding under the bypass id writes a key nothing ever reads and
 * every assertion then times out against an empty feed, which is exactly what
 * the first run of this spec did.
 *
 * Polled rather than read once: the app writes the key during boot, so a single
 * `page.evaluate` immediately after navigation can race that write and return
 * `null` — or, worse, a stale value that sends the seed to a key nobody reads.
 */
async function resolveActiveUserId(page: Page): Promise<string> {
  await expect
    .poll(async () => page.evaluate(() => localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID')), {
      timeout: 20_000,
      message: 'the app never wrote OPENHUMAN_ACTIVE_USER_ID',
    })
    .not.toBeNull();

  const user = await page.evaluate(() => localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID'));
  if (!user) throw new Error('no active user id after polling');
  return user;
}

/**
 * Open the feed with `items` already persisted BEFORE the app mounts.
 *
 * This is deliberately a two-navigation sequence, and the reason is the whole
 * design of this file:
 *
 *   1. Boot once, only to learn the namespace (see `resolveActiveUserId`). The
 *      feed is empty on this pass and nothing is asserted against it.
 *   2. Register the seed as an `addInitScript`, which runs on every subsequent
 *      document BEFORE any application code, then navigate again. The second
 *      document therefore mounts with the blob already in `localStorage`.
 *
 * Writing the blob after the app has mounted does not work: redux-persist
 * rehydrates once per page context, so a post-boot write is simply never read.
 * `addInitScript` is what makes the seed visible to rehydration — a plain
 * `page.evaluate` + `goto` is not equivalent, because the app has already
 * mounted and read its initial state by then.
 *
 * The blob shape is redux-persist's: each whitelisted field is its own JSON
 * string inside the outer object (`store/index.ts:144-149`).
 */
async function openFeedWith(page: Page, items: SeedItem[]): Promise<void> {
  await bootAuthenticatedPage(page, USER, '/notifications');
  await waitForAppReady(page);
  const user = await resolveActiveUserId(page);

  // One-shot. `addInitScript` runs before EVERY new document, `page.reload()`
  // included, so without the marker the reload in "read state survives a reload"
  // would re-apply the original payload — restoring `read: false` over the state
  // that test just marked read, and destroying the very thing it asserts. The
  // marker lives in localStorage, so it survives the reload alongside the
  // persisted blob and the seed applies to the first seeded navigation only.
  await page.addInitScript(
    ({ key, marker, payload }) => {
      if (window.localStorage.getItem(marker)) return;
      window.localStorage.setItem(marker, '1');
      const raw = window.localStorage.getItem(key);
      // Merge onto whatever the first boot wrote. When the key does not exist
      // yet, a blob carrying only `items` is not what redux-persist writes —
      // the real one also has `preferences` and `_persist`, and a missing
      // `_persist` is what tells it the blob is not rehydratable. Supply both
      // defaults so a fresh namespace behaves like a returning user's.
      const blob: Record<string, string> = raw
        ? (JSON.parse(raw) as Record<string, string>)
        : {
            preferences: JSON.stringify({
              messages: true,
              agents: true,
              skills: true,
              system: true,
              meetings: true,
              reminders: true,
              important: true,
            }),
            _persist: JSON.stringify({ version: -1, rehydrated: true }),
          };
      blob.items = JSON.stringify(payload);
      window.localStorage.setItem(key, JSON.stringify(blob));
    },
    { key: `${user}:persist:notifications`, marker: 'pw:notif-seed-applied', payload: items }
  );

  // A fresh document: the init script above runs first, so the store rehydrates
  // with the seed instead of the empty blob the previous boot left behind.
  await page.goto('/#/notifications');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('system-events-section')).toBeVisible({ timeout: 20_000 });
}

const feed = (page: Page) => page.getByTestId('system-events-section');
const rows = (page: Page) => feed(page).getByTestId('notification-item');

test.describe.skip('Notifications — the system-events feed renders what was stored', () => {
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

test.describe.skip('Notifications — filtering is single-select and actually filters', () => {
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

test.describe.skip('Notifications — read state', () => {
  test('clicking an unread item marks it read and drops the unread count', async ({ page }) => {
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);
    await expect(rows(page)).toHaveCount(2, { timeout: 20_000 });

    // The header reports the unread count; it is the user-visible signal that
    // the click did anything.
    await expect(page.getByText(/2 unread/i)).toBeVisible();

    await rows(page).first().click();

    await expect(page.getByText(/1 unread/i)).toBeVisible({ timeout: 10_000 });
  });

  test('Mark all read clears the count and disables itself', async ({ page }) => {
    await openFeedWith(page, [
      item('n-agents-1', 'agents', 'Agent finished a task'),
      item('n-system-1', 'system', 'Core restarted'),
    ]);

    const markAll = page.getByRole('button', { name: /mark all read/i });
    await expect(markAll).toBeEnabled({ timeout: 20_000 });

    await markAll.click();

    await expect(page.getByText(/unread/i)).toHaveCount(0, { timeout: 10_000 });
    await expect(markAll).toBeDisabled();
  });

  test('Mark all read is already disabled when nothing is unread', async ({ page }) => {
    await openFeedWith(page, [item('n-agents-1', 'agents', 'Agent finished a task', true)]);

    await expect(rows(page)).toHaveCount(1, { timeout: 20_000 });
    await expect(page.getByRole('button', { name: /mark all read/i })).toBeDisabled();
  });

  test('read state survives a reload', async ({ page }) => {
    // The slice is persisted, so marking read must outlive the page. If it does
    // not, the feed re-accuses the user of everything they just dismissed.
    await openFeedWith(page, [item('n-agents-1', 'agents', 'Agent finished a task')]);

    await page.getByRole('button', { name: /mark all read/i }).click();
    await expect(page.getByRole('button', { name: /mark all read/i })).toBeDisabled({
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

    await expect(page.getByRole('button', { name: /mark all read/i })).toBeDisabled({
      timeout: 20_000,
    });
  });
});
