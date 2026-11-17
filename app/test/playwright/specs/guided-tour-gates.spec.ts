import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function armWalkthrough(page: Page): Promise<void> {
  await page.evaluate(() => {
    localStorage.removeItem('openhuman:walkthrough_completed');
    localStorage.setItem('openhuman:walkthrough_pending', 'true');
  });
  // Mount Joyride fresh so a previously completed run cannot retain its old
  // step index when the walkthrough is armed again.
  await page.reload();
  await waitForAppReady(page);
}

async function tooltip(page: Page): Promise<Locator> {
  return page.locator('[role="alertdialog"]');
}

async function clickTourNext(page: Page, currentIndex: number): Promise<void> {
  const panel = await tooltip(page);
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(`${currentIndex + 1} of 7`);
  await panel.getByRole('button', { name: /Next|Let's go!/ }).click();
  await expect(panel).toContainText(`${currentIndex + 2} of 7`);
}

test.describe('Guided tour gates', () => {
  test.beforeEach(async ({ page }) => {
    // Tour coverage does not exercise the auth callback. Seed the authenticated
    // core state directly so callback timing cannot obscure walkthrough failures.
    await bootAuthenticatedPage(page, 'pw-guided-tour-user', '/home', {
      markWalkthroughComplete: false,
    });
    await dismissWalkthroughIfPresent(page);
    await page.goto('/#/home');
    await waitForAppReady(page);
  });

  test('follows the current seven-step route and ends with its seeded welcome note', async ({
    page,
  }) => {
    await armWalkthrough(page);

    const steps = [
      { hash: /#\/chat$/, target: 'chat-agent-panel' },
      { hash: /#\/connections\?tab=composio$/, target: 'skills-grid' },
      { hash: /#\/connections\?tab=channels$/, target: 'skills-channels' },
      { hash: /#\/settings\/account$/, target: 'settings-menu' },
      { hash: /#\/chat$/, target: 'tab-chat' },
      { hash: /#\/chat$/, target: 'tab-connections' },
      { hash: /#\/chat$/, target: 'chat-agent-panel' },
    ];
    for (let index = 0; index < steps.length; index += 1) {
      const step = steps[index];
      const panel = await tooltip(page);
      await expect(panel).toBeVisible();
      await expect(panel).toContainText(`${index + 1} of 7`);
      await expect.poll(() => page.evaluate(() => window.location.hash)).toMatch(step.hash);
      await expect(page.locator(`[data-walkthrough="${step.target}"]`)).toBeVisible();
      if (index < steps.length - 1) await clickTourNext(page, index);
    }
    const finalPanel = await tooltip(page);
    await expect(finalPanel).toContainText("You're all set!");
    await expect(finalPanel.getByRole('button', { name: /Skip/i })).toHaveCount(0);
    await expect(page.getByText(/Hey! Welcome to OpenHuman/)).toBeVisible();
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

  test('pending walkthrough resumes after reload', async ({ page }) => {
    await page.evaluate(() => {
      localStorage.removeItem('openhuman:walkthrough_completed');
      localStorage.setItem('openhuman:walkthrough_pending', 'true');
    });

    await page.reload();
    await waitForAppReady(page);

    const panel = await tooltip(page);
    await expect(panel).toBeVisible();
    await expect(panel.getByText('1 of 7')).toBeVisible();
    await expect(page.locator('[data-walkthrough="chat-agent-panel"]')).toBeVisible();
  });
});
