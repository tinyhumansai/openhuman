import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage } from '../helpers/core-rpc';

// The command-palette input is cmdk's `Command.Input`, which renders with a
// `cmdk-input` attribute. We target that specifically rather than a generic
// `input[role="combobox"]` — other pages (e.g. Settings, which now has a global
// search bar) legitimately render their own combobox, so the broad selector
// would no longer uniquely identify the palette.
const PALETTE_INPUT = 'input[cmdk-input]';

async function openPalette(page: import('@playwright/test').Page) {
  const shortcut = process.platform === 'darwin' ? 'Meta+K' : 'Control+K';
  await page.keyboard.press(shortcut);
  await expect(page.locator(PALETTE_INPUT)).toBeVisible();
}

test.describe('Command Palette', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-command-palette-user');
  });

  test('opens via mod+K, navigates to settings, and closes', async ({ page }) => {
    await openPalette(page);

    const input = page.locator(PALETTE_INPUT);
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
    await expect(page.getByText('Go to Knowledge & Memory')).toBeVisible();
    await expect(page.getByText('Go to Connections')).toBeVisible();
    await expect(page.getByText('Open Settings')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.locator(PALETTE_INPUT)).toHaveCount(0);
  });

  test('keeps the shortcut listener active after closing and reopening', async ({ page }) => {
    await openPalette(page);
    await page.keyboard.press('Escape');
    await expect(page.locator(PALETTE_INPUT)).toHaveCount(0);
    await openPalette(page);
    await expect(page.getByText('Go to Chat')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator(PALETTE_INPUT)).toHaveCount(0);
  });

  test('opens keyboard shortcuts with mod+/ and shows registered actions', async ({ page }) => {
    const shortcut = process.platform === 'darwin' ? 'Meta+/' : 'Control+/';
    await page.keyboard.press(shortcut);
    const list = page.getByTestId('keyboard-shortcuts-list');
    await expect(list).toBeVisible();
    await expect(list).toContainText('New Chat');
    await expect(list).toContainText('Toggle Sidebar');
    await expect(list).toContainText('Navigation');
    await page.keyboard.press('Escape');
    await expect(list).toHaveCount(0);
  });

  test('opens keyboard shortcuts with ? when focus is outside a text field', async ({ page }) => {
    await page.locator('body').click({ position: { x: 1, y: 1 } });
    await page.keyboard.press('?');
    const list = page.getByTestId('keyboard-shortcuts-list');
    await expect(list).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(list).toHaveCount(0);
  });
});
