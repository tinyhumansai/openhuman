import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Rewards and invites survive a real browser reload.
 *
 * `rewards-progression-persistence.spec.ts` already covers a "simulated restart"
 * — but it does that with `page.goto('/#/home')` and back, which is a **remount
 * inside the same JS context**. Redux, every in-memory cache and the whole
 * module graph survive it. A user pressing Cmd-R does something categorically
 * different: the context is destroyed and the page rebuilds from persisted
 * storage plus whatever it re-fetches.
 *
 * That gap is exactly where "works until you refresh" bugs live, and nothing in
 * the suite crosses it for these two surfaces. `top-level-functional-flows.spec.ts`
 * covers the invite copy/redeem interactions but never reloads either.
 *
 * These specs therefore assert the same user-visible facts on both sides of a
 * `page.reload()`, and deliberately do not re-test the interactions the existing
 * specs already own.
 */

// Two full boots plus a hard reload per test. The default 60s budget is enough
// on an idle machine but not when several e2e sessions share it — the first
// green run had a test at 51.0s, and the next run tipped all three over the
// wall with `Test timeout of 60000ms exceeded` rather than any assertion
// failing. Raised here rather than in the shared playwright.config.ts.
test.describe.configure({ timeout: 150_000 });

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function mockAdmin(path: string, body: unknown): Promise<void> {
  // Errors propagate on purpose. Swallowing them lets a failed reset leave the
  // shared mock stale, and the persistence result this file reports would then
  // be about the previous test's fixture rather than this one's.
  const response = await fetch(`${MOCK_ADMIN_BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(`mock admin ${path} failed with HTTP ${response.status}`);
  }
}

const resetMock = () => mockAdmin('/__admin/reset', {});
const setBehavior = (behavior: Record<string, unknown>) =>
  mockAdmin('/__admin/behavior', { behavior });
/** The rewards spec's admin shape is flat key/value rather than a `behavior` object. */
const setBehaviorKV = (key: string, value: string) =>
  mockAdmin('/__admin/behavior', { key, value });

async function settle(page: Page): Promise<void> {
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Rewards — progression survives a hard reload', () => {
  test('the same unlocked summary and usage metrics render after Cmd-R', async ({ page }) => {
    await resetMock();
    await setBehaviorKV('rewardsScenario', 'high_usage');
    await setBehaviorKV('rewardsLastSyncedAt', '2026-04-28T09:00:00.000Z');

    await bootAuthenticatedPage(page, 'pw-rewards-reload', '/rewards?view=main');
    await settle(page);

    // 60s, not the default: this is the first test in the file, so it pays the
    // app's cold boot AND the first rewards fetch. Observed passing at 51s on an
    // idle machine and missing first paint entirely under fleet load.
    await expect(page.getByText('Your Progress')).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText('Activity streak')).toBeVisible();
    await expect(page.getByText('14 days')).toBeVisible();
    await expect(page.getByText('12,500,000')).toBeVisible();

    // The real thing: destroy the JS context, not just the route.
    await page.reload();
    await settle(page);

    await expect(page.getByText('Your Progress')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText('Activity streak')).toBeVisible();
    await expect(page.getByText('14 days')).toBeVisible();
    await expect(page.getByText('12,500,000')).toBeVisible();
  });

  test('a reload lands back on the rewards route rather than the default surface', async ({
    page,
  }) => {
    // A deep link that does not survive a refresh is a broken bookmark: the
    // user reloads to check something and is silently moved elsewhere.
    await resetMock();
    await setBehaviorKV('rewardsScenario', 'high_usage');
    // Both keys, matching rewards-progression-persistence.spec.ts: seeding the
    // scenario without a sync timestamp leaves the page without a progress
    // summary to render, and the failure looks like a routing bug.
    await setBehaviorKV('rewardsLastSyncedAt', '2026-04-28T09:00:00.000Z');

    await bootAuthenticatedPage(page, 'pw-rewards-reload-route', '/rewards?view=main');
    await settle(page);
    await expect(page.getByText('Your Progress')).toBeVisible({ timeout: 20_000 });

    await page.reload();
    await settle(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/rewards/);
    await expect(page.getByText('Your Progress')).toBeVisible();
  });
});

test.describe('Invites — the code survives a hard reload', () => {
  test('the rendered invite code is still there after Cmd-R', async ({ page }) => {
    const inviteCode = `PW${Date.now().toString().slice(-6)}`;
    await resetMock();
    await setBehavior({
      inviteCodes: JSON.stringify([
        { _id: 'invite-pw-reload', code: inviteCode, currentUses: 0, maxUses: 1, usageHistory: [] },
      ]),
    });

    await bootAuthenticatedPage(page, 'pw-invites-reload', '/invites');
    await settle(page);
    await expect(page.getByText(inviteCode)).toBeVisible({ timeout: 20_000 });

    await page.reload();
    await settle(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/invites/);
    await expect(page.getByText(inviteCode)).toBeVisible({ timeout: 20_000 });
  });
});
