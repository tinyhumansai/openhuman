import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function armWalkthrough(page: Page): Promise<void> {
  await page.evaluate(() => {
    localStorage.removeItem('openhuman:walkthrough_completed');
    localStorage.setItem('openhuman:walkthrough_pending', 'true');
    window.dispatchEvent(new CustomEvent('walkthrough:restart'));
  });
}

async function tooltip(page: Page): Promise<Locator> {
  return page.locator('[role="alertdialog"]');
}

test.describe('Guided tour gates', () => {
  test.beforeEach(async ({ page }) => {
    // Tour coverage does not exercise the auth callback. Seed the authenticated
    // core state directly so callback timing cannot obscure walkthrough failures.
    await bootAuthenticatedPage(page, 'pw-guided-tour-user', '/home');
    await dismissWalkthroughIfPresent(page);
    await page.goto('/#/home');
    await waitForAppReady(page);
  });

  test('skip hides the tour and marks walkthrough complete', async ({ page }) => {
    await armWalkthrough(page);

    const panel = await tooltip(page);
    await expect(panel).toBeVisible();
    await panel.getByRole('button', { name: /Skip/ }).click();

    await expect(page.locator('#react-joyride-portal')).toHaveCount(0);
    await expect
      .poll(async () =>
        page.evaluate(() => ({
          completed: localStorage.getItem('openhuman:walkthrough_completed'),
          pending: localStorage.getItem('openhuman:walkthrough_pending'),
        }))
      )
      .toEqual({ completed: 'true', pending: null });
  });
});
