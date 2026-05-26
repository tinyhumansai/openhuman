import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage } from '../helpers/core-rpc';

async function openPalette(page: import('@playwright/test').Page) {
  const shortcut = process.platform === 'darwin' ? 'Meta+K' : 'Control+K';
  await page.keyboard.press(shortcut);
  await expect(page.locator('input[role="combobox"]')).toBeVisible();
}

test.describe('Command Palette', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-command-palette-user');
  });

  test('opens via mod+K, navigates to settings, and closes', async ({ page }) => {
    await openPalette(page);

    const input = page.locator('input[role="combobox"]');
    await input.fill('settings');
    await page.keyboard.press('Enter');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/settings/);
    await expect(input).toHaveCount(0);
  });

  test('lists the seed navigation actions and closes on Escape', async ({ page }) => {
    await openPalette(page);

    await expect(page.getByText('Go Home')).toBeVisible();
    await expect(page.getByText('Go to Chat')).toBeVisible();
    await expect(page.getByText('Go to Intelligence')).toBeVisible();
    await expect(page.getByText('Go to Skills')).toBeVisible();
    await expect(page.getByText('Open Settings')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.locator('input[role="combobox"]')).toHaveCount(0);
  });
});
