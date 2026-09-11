import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Theme switching, asserted where it actually lands: on the document.
 *
 * `ThemeProvider.applyTheme` (`providers/ThemeProvider.tsx:38-60`) does three
 * separate things — toggles the `dark` class on `<html>`, sets
 * `style.colorScheme`, and writes the palette as `--*` custom properties. A
 * jsdom test can observe the class, but the custom properties and the resolved
 * `color-scheme` are only meaningful in a real engine, and "the theme visibly
 * applied" means all three moved together.
 *
 * `/settings/appearance` hosts the theme studio (the standalone
 * `/settings/theme` route redirects here). Its two controls are separate and
 * compose: a VARIANT toggle (Light / Dark / Auto) and a FAMILY grid (Classic,
 * Ocean, Sepia, Matrix, HAL 9000 — `lib/theme/presets.ts:342`). Choosing
 * "Dark" is the variant toggle; choosing "HAL 9000" is a family. An earlier
 * version of this spec looked for a button named "Dark" in the family grid and
 * timed out, because no such button exists.
 */

/** The Light / Dark / Auto toggle, scoped so a family name cannot match it. */
const variant = (page: Page, label: string) =>
  page.getByLabel('Theme variant').getByText(label, { exact: true });

/** A palette family tile in the grid. */
const family = (page: Page, name: string) => page.getByRole('button', { name, exact: true });

/** What the document actually holds after a theme is applied. */
function documentTheme(page: Page) {
  return page.evaluate(() => {
    const root = document.documentElement;
    const styles = getComputedStyle(root);
    return {
      dark: root.classList.contains('dark'),
      colorScheme: root.style.colorScheme,
      surface: styles.getPropertyValue('--surface').trim(),
      content: styles.getPropertyValue('--content').trim(),
    };
  });
}

async function openAppearance(page: Page) {
  // Navigate explicitly even though beforeEach booted here: on a cold first
  // test the '/home' -> '/chat' redirect can still be in flight and win the
  // race, leaving the chat surface behind. Re-navigating settles it.
  await page.goto('/#/settings/appearance');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Appearance', {
    timeout: 30_000,
  });
}

test.describe('Appearance — theme switching', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-theme', '/settings/appearance');
    await openAppearance(page);
  });

  test('choosing Dark sets the dark class and color-scheme on <html>', async ({ page }) => {
    await variant(page, 'Dark').click();

    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);
    expect((await documentTheme(page)).colorScheme).toBe('dark');
  });

  test('choosing Light clears them again', async ({ page }) => {
    await variant(page, 'Dark').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);

    await variant(page, 'Light').click();

    await expect.poll(async () => (await documentTheme(page)).dark).toBe(false);
    expect((await documentTheme(page)).colorScheme).toBe('light');
  });

  // The class alone does not prove the palette applied — a theme that set the
  // class and no custom properties would look unchanged to the user.
  test('the palette custom properties change with the theme', async ({ page }) => {
    await variant(page, 'Light').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(false);
    const light = await documentTheme(page);

    await variant(page, 'Dark').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);
    const dark = await documentTheme(page);

    expect(light.surface).not.toBe('');
    expect(dark.surface).not.toBe('');
    expect(dark.surface).not.toBe(light.surface);
    expect(dark.content).not.toBe(light.content);
  });

  test('a named palette family applies its own surface, distinct from Classic', async ({
    page,
  }) => {
    await variant(page, 'Dark').click();
    await family(page, 'Classic').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);
    const classicDark = await documentTheme(page);

    await family(page, 'HAL 9000').click();
    await expect
      .poll(async () => (await documentTheme(page)).surface)
      .not.toBe(classicDark.surface);

    // The variant is unchanged — the palette moved, not light/dark mode.
    expect((await documentTheme(page)).dark).toBe(true);
  });

  test('the chosen family is marked pressed, and only that one', async ({ page }) => {
    await family(page, 'Ocean').click();

    await expect(family(page, 'Ocean')).toHaveAttribute('aria-pressed', 'true');
    await expect(family(page, 'Sepia')).toHaveAttribute('aria-pressed', 'false');

    await family(page, 'Sepia').click();

    await expect(family(page, 'Sepia')).toHaveAttribute('aria-pressed', 'true');
    await expect(family(page, 'Ocean')).toHaveAttribute('aria-pressed', 'false');
  });

  // Variant and family are independent axes: switching palette must not flip
  // light/dark, and vice versa.
  test('family and variant compose without overriding each other', async ({ page }) => {
    await variant(page, 'Dark').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);

    await family(page, 'Ocean').click();
    const oceanDark = await documentTheme(page);
    expect(oceanDark.dark).toBe(true);

    await variant(page, 'Light').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(false);

    // Still Ocean, now light — so the surface must differ from Ocean dark.
    await expect(family(page, 'Ocean')).toHaveAttribute('aria-pressed', 'true');
    expect((await documentTheme(page)).surface).not.toBe(oceanDark.surface);
  });

  test('the theme survives a reload', async ({ page }) => {
    await variant(page, 'Dark').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);
    const before = await documentTheme(page);

    await page.reload();
    await waitForAppReady(page);

    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);
    expect((await documentTheme(page)).surface).toBe(before.surface);
  });

  test('the theme applies outside settings too, not just on this panel', async ({ page }) => {
    await variant(page, 'Dark').click();
    await expect.poll(async () => (await documentTheme(page)).dark).toBe(true);

    // The class lives on <html>, so leaving the panel must not drop it — a
    // theme scoped to the settings subtree would be the bug this catches.
    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Privacy');

    expect((await documentTheme(page)).dark).toBe(true);
  });
});
