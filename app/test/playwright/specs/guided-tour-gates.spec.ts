import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
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

async function clickTourNext(page: Page): Promise<void> {
  const panel = await tooltip(page);
  await expect(panel).toBeVisible();
  await panel.getByRole('button', { name: /Next|Let's go!/ }).click();
}

test.describe('Guided tour gates', () => {
  test.beforeEach(async ({ page }) => {
    await bootRuntimeReadyGuestPage(page);
    await signInViaBypassUser(page, 'pw-guided-tour-user');
    await dismissWalkthroughIfPresent(page);
    await page.goto('/#/home');
    await waitForAppReady(page);
  });

  test('tour starts from home and can navigate forward to the skills step', async ({ page }) => {
    await armWalkthrough(page);

    const panel = await tooltip(page);
    await expect(panel).toBeVisible();
    await expect(page.locator('[data-walkthrough="home-card"]')).toBeVisible();

    await clickTourNext(page);
    await expect(page.locator('[data-walkthrough="home-cta"]')).toBeVisible();

    await clickTourNext(page);
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/chat');
    await expect(page.locator('[data-walkthrough="chat-agent-panel"]')).toBeVisible();

    await clickTourNext(page);
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/skills');
    await expect(page.locator('[data-walkthrough="skills-grid"]')).toBeVisible();
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

  test.skip('pending walkthrough resumes after reload', async ({ page }) => {
    await page.evaluate(() => {
      localStorage.removeItem('openhuman:walkthrough_completed');
      localStorage.setItem('openhuman:walkthrough_pending', 'true');
    });

    await page.reload();
    await waitForAppReady(page);

    const panel = await tooltip(page);
    await expect(panel).toBeVisible();
    await expect(panel.getByText('1 of 10')).toBeVisible();
    await expect(page.locator('[data-walkthrough="home-card"]')).toBeVisible();
  });
});
