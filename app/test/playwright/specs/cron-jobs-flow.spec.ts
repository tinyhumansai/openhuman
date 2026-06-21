import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const MORNING_BRIEFING = 'morning_briefing';

async function openCronJobsPanel(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/#/settings/cron-jobs');
  await waitForAppReady(page);
  // Panel title dropped in the PanelPage migration; the panel test id and its
  // Scheduled Jobs section confirm it mounted.
  await expect(page.getByTestId('cron-jobs-panel')).toBeVisible();
  await expect(page.getByText('Scheduled Jobs').first()).toBeVisible();
}

test.describe('Cron jobs settings panel', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-cron-jobs-flow', '/home');
  });

  test('chat surface is reachable after login', async ({ page }) => {
    // Home folded into the unified chat surface: post-login landing is /chat.
    await waitForAppReady(page);
    const text = await page.locator('#root').innerText();
    expect(['New Conversation', 'Threads'].some(marker => text.includes(marker))).toBe(true);
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
