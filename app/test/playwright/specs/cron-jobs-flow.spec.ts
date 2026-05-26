import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const MORNING_BRIEFING = 'morning_briefing';

async function openCronJobsPanel(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/#/settings/cron-jobs');
  await waitForAppReady(page);
  await expect(page.getByRole('heading', { name: 'Cron Jobs', exact: true })).toBeVisible();
  await expect(page.getByText('Scheduled Jobs').first()).toBeVisible();
  await expect(page.getByTestId('cron-jobs-panel')).toBeVisible();
}

test.describe('Cron jobs settings panel', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-cron-jobs-flow', '/home');
  });

  test('home screen is reachable after login', async ({ page }) => {
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  test('cron jobs panel renders in the browser lane and surfaces the current fallback state', async ({
    page,
  }) => {
    await openCronJobsPanel(page);
    const text = await page.locator('#root').innerText();
    expect(
      [
        'Failed to load core cron jobs: Not running in Tauri',
        'No core cron jobs found.',
        MORNING_BRIEFING,
      ].some(marker => text.includes(marker))
    ).toBe(true);
  });

  test('refresh action is visible in the cron jobs panel', async ({ page }) => {
    await openCronJobsPanel(page);
    await expect(page.getByRole('button', { name: 'Refresh Cron Jobs' })).toBeVisible();
  });
});
