import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Theme Studio import validation (#5946 / #5901), asserted in a real engine.
 *
 * `handleImport` used to gate on `typeof parsed.colors !== 'object'`. Both
 * `typeof null` and `typeof []` are `'object'`, so a paste carrying
 * `"colors": null` or `"colors": []` passed the check; `{ ...null }` and
 * `{ ...[] }` each spread to `{}`, and the malformed paste was stored as a
 * theme. A non-string token value got further still — `swatchChannels` falls
 * back only on null/undefined, so `{"surface": 42}` reached `channelsToCss`,
 * which calls `.trim()` on it and throws, crashing the panel on a theme that
 * was by then already in the store.
 *
 * The panel is reached at `/settings/appearance`, which embeds
 * `ThemeStudioPanel` (`AppearancePanel.tsx:98`); the standalone
 * `/settings/theme` route redirects there.
 *
 * The observable signal for accept-vs-reject is the textarea. `handleImport`
 * clears `importText` only on the success path, and sets `importError` only on
 * the failure path — so "the error is shown AND the pasted text is still
 * there" is a state the accepting build cannot produce.
 */

const importBox = (page: Page) => page.getByLabel('Import theme');
const importButton = (page: Page) => page.getByRole('button', { name: 'Import', exact: true });
const importError = (page: Page) => page.getByText('Could not parse that theme JSON.');

async function openAppearance(page: Page) {
  await page.goto('/#/settings/appearance');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Appearance', {
    timeout: 20_000,
  });
  await expect(importBox(page)).toBeVisible({ timeout: 20_000 });
}

/** Paste `json` into the import box and click Import. */
async function attemptImport(page: Page, json: string) {
  await importBox(page).fill(json);
  await importButton(page).click();
}

test.describe('Theme Studio — import validation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w8-theme-import', '/settings/appearance');
    await openAppearance(page);
  });

  // Each of these reached the store before #5946. `null` and `[]` are the two
  // that `typeof x === 'object'` waves through; the rest are token values that
  // survive to `channelsToCss` and throw there rather than here.
  const malformed: ReadonlyArray<readonly [string, string]> = [
    ['null colors', JSON.stringify({ name: 'Malformed', isDark: false, colors: null })],
    ['an array of colors', JSON.stringify({ name: 'Malformed', isDark: false, colors: [] })],
    [
      'a numeric token value',
      JSON.stringify({ name: 'Malformed', isDark: false, colors: { surface: 42 } }),
    ],
    [
      'a null token value',
      JSON.stringify({ name: 'Malformed', isDark: false, colors: { surface: null } }),
    ],
    [
      'one bad value among good ones',
      JSON.stringify({
        name: 'Malformed',
        isDark: false,
        colors: { surface: '1 2 3', content: 7 },
      }),
    ],
  ];

  for (const [label, json] of malformed) {
    test(`refuses a theme with ${label}`, async ({ page }) => {
      await attemptImport(page, json);

      await expect(importError(page)).toBeVisible();
      // The success path clears the box. Still holding the paste is what
      // separates "refused" from "accepted and stored".
      await expect(importBox(page)).toHaveValue(json);
    });
  }

  test('still accepts a theme carrying a single colour token', async ({ page }) => {
    const valid = JSON.stringify({ name: 'Minimal', isDark: false, colors: { surface: '1 2 3' } });

    await attemptImport(page, valid);

    await expect(importError(page)).toBeHidden();
    await expect(importBox(page)).toHaveValue('');
  });

  test('still accepts a theme whose colors object is empty', async ({ page }) => {
    // Deliberately allowed, and the reason is load-bearing: CLASSIC_LIGHT and
    // CLASSIC_DARK both carry `colors: {}` (`lib/theme/presets.ts`), inheriting
    // the base stylesheet tokens and carrying their meaning in `isDark`.
    // Rejecting `{}` would break the panel's own export -> import round trip
    // for the two most common themes, and refuse font- or backdrop-only themes.
    const emptyColors = JSON.stringify({ name: 'Inherits', isDark: true, colors: {} });

    await attemptImport(page, emptyColors);

    await expect(importError(page)).toBeHidden();
    await expect(importBox(page)).toHaveValue('');
  });
});
