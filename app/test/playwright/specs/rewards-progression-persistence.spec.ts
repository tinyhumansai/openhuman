import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function gotoRewards(page: import('@playwright/test').Page, userId: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, '/rewards');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByText('Your Progress')).toBeVisible();
}

async function rewardsRequestCount(): Promise<number> {
  const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const json = (await res.json()) as { data?: Array<{ method: string; url: string }> };
  const log = json.data ?? [];
  return log.filter(r => r.method === 'GET' && /^\/rewards\/me/.test(r.url)).length;
}

test.describe('Rewards Progression Persistence', () => {
  test('message-driven progress is reflected in the unlocked summary', async ({ page }) => {
    await resetMock();
    await setMockBehavior('rewardsScenario', 'high_usage');
    await gotoRewards(page, 'pw-rewards-progress-message');

    await expect(page.getByText('3 of 3 achievements unlocked')).toBeVisible();
    await expect(page.getByText('7-Day Streak')).toBeVisible();
    await expect(page.getByText('Discord Member')).toBeVisible();
    await expect(page.getByText('Pro Supporter')).toBeVisible();
  });

  test('usage metrics render current streak and cumulative tokens', async ({ page }) => {
    await resetMock();
    await setMockBehavior('rewardsScenario', 'high_usage');
    await gotoRewards(page, 'pw-rewards-progress-metrics');

    await expect(page.getByText('Current streak')).toBeVisible();
    await expect(page.getByText('14')).toBeVisible();
    await expect(page.getByText('Cumulative tokens')).toBeVisible();
    await expect(page.getByText('12,500,000')).toBeVisible();
  });

  test('state persists across a simulated restart / remount', async ({ page }) => {
    await resetMock();
    await setMockBehavior('rewardsScenario', 'high_usage');
    await setMockBehavior('rewardsLastSyncedAt', '2026-04-28T09:00:00.000Z');
    await gotoRewards(page, 'pw-rewards-progress-persist');

    await expect(page.getByText('Current streak')).toBeVisible();
    await expect(page.getByText('14')).toBeVisible();
    await expect(page.getByText('12,500,000')).toBeVisible();

    await setMockBehavior('rewardsScenario', 'post_restart');
    await setMockBehavior('rewardsLastSyncedAt', '2026-04-28T10:30:00.000Z');

    await page.goto('/#/home');
    await waitForAppReady(page);
    await page.goto('/#/rewards');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await expect(page.getByText('Your Progress')).toBeVisible();

    await expect(page.getByText('3 of 3 achievements unlocked')).toBeVisible();
    await expect(page.getByText('Current streak')).toBeVisible();
    await expect(page.getByText('14')).toBeVisible();
    await expect(page.getByText('12,500,000')).toBeVisible();
    await expect.poll(() => rewardsRequestCount()).toBeGreaterThanOrEqual(2);
  });
});
