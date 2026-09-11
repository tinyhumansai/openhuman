import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Privacy Mode — the data-egress posture, asserted against the CORE.
 *
 * This control decides what leaves the user's machine (#4435): `local_only`,
 * `standard`, or `sensitive`. Believing you are in `local_only` when the core
 * still has `standard` is the whole risk, and it is not observable from the DOM
 * — the radio can show the click while the write failed or never happened.
 *
 * So every assertion here reads `openhuman.config_get_privacy_mode` back from
 * the core and compares. `settings-account-preferences.spec.ts` persists the
 * *analytics* toggle; nothing drives the egress mode, in this lane or any other.
 */

type PrivacyMode = 'local_only' | 'standard' | 'sensitive';

async function coreMode(): Promise<string | undefined> {
  const res = await callCoreRpc<{ result?: { mode?: string } }>(
    'openhuman.config_get_privacy_mode',
    {}
  );
  return res.result?.mode;
}

async function setCoreMode(mode: PrivacyMode): Promise<void> {
  await callCoreRpc('openhuman.config_set_privacy_mode', { mode });
}

/** The radio input — queryable for state, but `sr-only`, so never clicked. */
const option = (page: Page, mode: PrivacyMode) => page.getByTestId(`privacy-mode-option-${mode}`);

/**
 * The visible label that wraps each radio. The input itself carries `sr-only`
 * (`PrivacyModeSection.tsx:120`), so clicking it times out — a user clicks the
 * card, and the `htmlFor` association is what activates the radio.
 */
const choose = (page: Page, mode: PrivacyMode) =>
  page.locator(`label[for="privacy-mode-option-${mode}-input"]`);

async function openPrivacy(page: Page) {
  await page.goto('/#/settings/privacy');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Privacy', { timeout: 30_000 });
  await expect(page.getByTestId('privacy-mode-options')).toBeVisible({ timeout: 30_000 });
}

test.describe('Privacy mode — the selector reflects the core', () => {
  test.beforeEach(async ({ page }) => {
    // Start from a known posture so "it changed" means something.
    await setCoreMode('standard');
    await bootAuthenticatedPage(page, 'pw-w1-privacy', '/settings/privacy');
    await openPrivacy(page);
  });

  test('loads the mode the core actually holds, not a default', async ({ page }) => {
    await expect(option(page, 'standard')).toBeChecked();
    await expect(option(page, 'local_only')).not.toBeChecked();
    await expect(option(page, 'sensitive')).not.toBeChecked();
  });

  test('a mode set outside the UI is reflected on next open', async ({ page }) => {
    await setCoreMode('sensitive');

    // Leave and come back: same hash would be a router no-op, so hop.
    await page.goto('/#/settings/security');
    await waitForAppReady(page);
    await openPrivacy(page);

    await expect(option(page, 'sensitive')).toBeChecked();
  });
});

test.describe('Privacy mode — selecting a posture reaches the core', () => {
  test.beforeEach(async ({ page }) => {
    await setCoreMode('standard');
    await bootAuthenticatedPage(page, 'pw-w1-privacy-write', '/settings/privacy');
    await openPrivacy(page);
  });

  test('choosing local_only writes local_only to the core', async ({ page }) => {
    await choose(page, 'local_only').click();

    await expect(option(page, 'local_only')).toBeChecked();
    // The assertion that matters: the CORE agrees. A UI-only change here would
    // leave the user believing their data stays on the machine when it does not.
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('local_only');
  });

  test('choosing sensitive writes sensitive to the core', async ({ page }) => {
    await choose(page, 'sensitive').click();

    await expect(option(page, 'sensitive')).toBeChecked();
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('sensitive');
  });

  test('switching postures twice leaves the core on the last one', async ({ page }) => {
    await choose(page, 'local_only').click();
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('local_only');

    await choose(page, 'sensitive').click();
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('sensitive');

    await expect(option(page, 'sensitive')).toBeChecked();
    await expect(option(page, 'local_only')).not.toBeChecked();
  });

  test('the chosen posture survives a reload', async ({ page }) => {
    await choose(page, 'local_only').click();
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('local_only');

    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(option(page, 'local_only')).toBeChecked({ timeout: 30_000 });
    expect(await coreMode()).toBe('local_only');
  });

  test('re-selecting the current posture does not disturb it', async ({ page }) => {
    await choose(page, 'local_only').click();
    await expect.poll(coreMode, { timeout: 20_000 }).toBe('local_only');

    // The panel short-circuits when the mode is unchanged
    // (`PrivacyModeSection.tsx:69`). What must hold either way is that the
    // posture does not flip or clear.
    await choose(page, 'local_only').click();

    await expect(option(page, 'local_only')).toBeChecked();
    expect(await coreMode()).toBe('local_only');
  });

  test('the three postures are mutually exclusive in the DOM', async ({ page }) => {
    for (const mode of ['local_only', 'standard', 'sensitive'] as const) {
      await choose(page, mode).click();
      await expect.poll(coreMode, { timeout: 20_000 }).toBe(mode);

      // The radios are Radix items, so selection is `aria-checked` /
      // `data-state`, not the native `.checked` property — which is always
      // false here and would make this assertion vacuous.
      const checked = await page.evaluate(
        () =>
          Array.from(document.querySelectorAll('[data-testid^="privacy-mode-option-"]')).filter(
            el =>
              el.getAttribute('aria-checked') === 'true' ||
              el.getAttribute('data-state') === 'checked' ||
              (el as HTMLInputElement).checked === true
          ).length
      );
      expect(checked).toBe(1);
    }
  });
});
