import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function setRewardsScenario(value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'rewardsScenario', value }),
  });
}

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function gotoRewards(page: import('@playwright/test').Page, scenario: string) {
  await resetMock();
  await setRewardsScenario(scenario);
  await bootAuthenticatedPage(page, `pw-rewards-${scenario}`, '/rewards');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByText('Your Progress')).toBeVisible();
}

test.describe('Rewards Unlock Flow', () => {
  test('activity-based unlock surfaces the streak achievement', async ({ page }) => {
    await gotoRewards(page, 'activity_unlocked');
    await expect(page.getByText('1 of 3 achievements unlocked')).toBeVisible();
    await expect(page.getByText('7-Day Streak')).toBeVisible();
    await expect(page.getByText('Unlocked', { exact: true })).toBeVisible();
  });

  test('integration-based unlock reflects Discord membership', async ({ page }) => {
    await gotoRewards(page, 'integration_unlocked');
    await expect(page.getByText('1 of 3 achievements unlocked')).toBeVisible();
    await expect(page.getByText('Joined the server')).toBeVisible();
    await expect(page.getByText('Discord Member')).toBeVisible();
  });

  test('plan-based unlock surfaces the PRO achievement', async ({ page }) => {
    await gotoRewards(page, 'plan_unlocked');
    await expect(page.getByText('1 of 3 achievements unlocked')).toBeVisible();
    await expect(page.getByText('Pro Supporter')).toBeVisible();
    await expect(page.getByText('Discord not connected')).toBeVisible();
  });
});
