import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

/**
 * The "what leaves my computer" sheet, driven in a real browser.
 *
 * #5845 re-based this sheet from a hand-rolled portal onto the shared Radix
 * dialog (`components/ui/Dialog.tsx`), and no e2e lane ever opened it.
 *
 * A note on why that gap was invisible: `settings-channels-permissions.spec.ts`
 * asserts `getByText('What leaves your computer')` and reads as if it covers
 * this. It does not. That string is `privacy.whatLeavesComputer` (`en.ts:1389`),
 * a static label on the privacy panel. The sheet's trigger is
 * `privacy.whatLeaves.link.label` = 'What leaves my computer?' (`en.ts:4801`)
 * and its own headline is `WHAT_LEAVES_HEADLINE` (`whatLeavesItems.ts:30`).
 * Three different strings; only the panel label was ever asserted.
 *
 * WHAT IS AND IS NOT NEW HERE. The pre-#5845 sheet already rendered
 * `role="dialog"` with `aria-modal`, already closed on Escape (a `document`
 * keydown listener) and already closed on an overlay click (a full-bleed
 * `<button>`). Those three are *preserved* behaviour, not new, and a test
 * asserting only them would pass against the old component too. The one thing
 * the rewrite genuinely added is `aria-describedby` -> the subhead: the old
 * markup rendered that paragraph with no id and never referenced it, so a
 * screen reader announced the title alone. Every test below therefore asserts
 * the description wiring alongside whatever else it drives, so that each one
 * fails if the rewrite is reverted rather than merely re-passing.
 *
 * The sheet is reachable from exactly one place — `WelcomeStep.tsx:28`, the
 * onboarding welcome step — so these boot into onboarding rather than settings.
 */

const HEADLINE = 'Local by default. Cloud when you ask.';
const SUBHEAD = "For full transparency, here's exactly what does, and when.";
const TRIGGER = 'What leaves my computer?';

async function bootIntoOnboardingWelcome(page: Page, userId: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, '/home');
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/welcome');
  await waitForAppReady(page);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/onboarding/);
  await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible({ timeout: 20_000 });
}

const sheet = (page: Page) => page.getByRole('dialog');

/**
 * Open the sheet and assert the wiring #5845 introduced.
 *
 * The `aria-describedby` assertion lives here, in the shared open step, so it
 * guards every test in the file rather than only the one that names it.
 */
async function openSheetAndAssertDescribed(page: Page): Promise<void> {
  await expect(sheet(page)).toHaveCount(0);
  await page.getByRole('button', { name: TRIGGER }).click();
  await expect(sheet(page)).toBeVisible({ timeout: 10_000 });

  const describedBy = await sheet(page).getAttribute('aria-describedby');
  expect(
    describedBy,
    'the rewrite wired the dialog to its subhead; the hand-rolled sheet gave that paragraph no id at all'
  ).toBe('what-leaves-description');
  await expect(page.locator('#what-leaves-description')).toHaveText(SUBHEAD);
}

test.describe('Privacy — the "what leaves my computer" sheet', () => {
  test('opens as a described dialog carrying the honest list', async ({ page }) => {
    await bootIntoOnboardingWelcome(page, 'pw-what-leaves-open');
    await openSheetAndAssertDescribed(page);

    await expect(sheet(page)).toContainText(HEADLINE);

    // The three items are the point of the sheet — a dialog that renders its
    // chrome but drops its content would otherwise satisfy the assertions above.
    await expect(sheet(page)).toContainText('Cloud AI Inference');
    await expect(sheet(page)).toContainText('Third-party integrations');
    await expect(sheet(page)).toContainText('Crash Reports & Usage Data (opt-out)');
  });

  test('Escape closes it and returns the user to the step', async ({ page }) => {
    await bootIntoOnboardingWelcome(page, 'pw-what-leaves-escape');
    await openSheetAndAssertDescribed(page);

    // Preserved behaviour, not new — but it now belongs to Radix rather than a
    // hand-registered `document` listener, which is exactly the kind of thing a
    // library swap loses quietly.
    await page.keyboard.press('Escape');
    await expect(sheet(page)).toHaveCount(0, { timeout: 10_000 });
    await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible();
  });

  test('clicking the overlay outside the panel closes it', async ({ page }) => {
    await bootIntoOnboardingWelcome(page, 'pw-what-leaves-outside');
    await openSheetAndAssertDescribed(page);

    // Top-left of the viewport is overlay, never panel: the content is centred
    // and capped at `max-w-lg`.
    await page.mouse.click(5, 5);
    await expect(sheet(page)).toHaveCount(0, { timeout: 10_000 });
    await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible();
  });
});
