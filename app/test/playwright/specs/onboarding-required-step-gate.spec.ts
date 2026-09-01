import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

/**
 * The runtime-choice step: what it actually guarantees.
 *
 * This spec started from the wrong premise and the browser corrected it, which
 * is worth recording. `RuntimeChoiceStep.tsx:161` reads
 * `disabled={selected === null}`, so the step looks like a required-choice gate
 * with no coverage. It is not one: `:101` is
 * `useState<AiMode | null>('cloud')`, so **cloud is pre-selected on arrival**
 * and `selected` is never null through the UI. The `disabled` prop and the
 * `onClick={() => selected && onNext(selected)}` guard beside it are both dead
 * defensive code.
 *
 * The first run of this file asserted `toBeDisabled()` and failed with
 * `locator resolved to <button … aria-label="Continue with Simple"> unexpected
 * value "enabled"`. Those two tests were removed rather than reframed — a gate
 * that cannot engage is not a contract worth pinning, and asserting the dead
 * branch would have been a test that could never fail for the reason it named.
 *
 * What IS worth pinning, and is covered here: the step is immediately
 * actionable (a user can continue without hunting for a selection), the
 * Continue label names the choice they are about to commit to, and the two
 * options are mutually exclusive. `onboarding-modes.spec.ts` walks both happy
 * paths and `onboarding-config-functional.spec.ts` covers back navigation;
 * neither asserts any of the above.
 */

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  // Deliberately NOT swallowed. A failed reset leaves shared mock state from a
  // previous test, and onboarding then runs against a fixture nobody chose —
  // which surfaces as an unrelated assertion failure further down.
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
  if (!response.ok) {
    throw new Error(`mock admin reset failed with HTTP ${response.status}`);
  }
}

async function bootIntoOnboarding(page: Page, userId: string): Promise<void> {
  await resetMock();
  await bootAuthenticatedPage(page, userId, '/home');
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/welcome');
  await waitForAppReady(page);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/onboarding/);
}

async function reachRuntimeChoice(page: Page, userId: string): Promise<void> {
  await bootIntoOnboarding(page, userId);
  await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('onboarding-next-button').click();
  await expect(page.getByTestId('onboarding-runtime-choice-step')).toBeVisible({ timeout: 20_000 });
}

const nextButton = (page: Page) => page.getByTestId('onboarding-next-button');

test.describe('Onboarding — the runtime choice is a required step', () => {
  test('arrives with cloud pre-selected and immediately actionable', async ({ page }) => {
    // The default is what makes the step passable in one click. If it ever
    // regressed to `null`, Continue would be permanently disabled and the flow
    // would dead-end here — which is the failure the dead `disabled` prop was
    // presumably written against.
    await reachRuntimeChoice(page, 'pw-onboarding-default-cloud');

    await expect(page.getByTestId('onboarding-runtime-choice-cloud')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(nextButton(page)).toBeEnabled();
    await expect(nextButton(page)).toContainText(/simple|cloud/i);
  });

  test('re-selecting cloud keeps Continue enabled and named for it', async ({ page }) => {
    await reachRuntimeChoice(page, 'pw-onboarding-gate-cloud');

    await page.getByTestId('onboarding-runtime-choice-cloud').click();

    await expect(page.getByTestId('onboarding-runtime-choice-cloud')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(nextButton(page)).toBeEnabled();
    await expect(nextButton(page)).toContainText(/simple|cloud/i);
  });

  test('choosing custom enables Continue and names the other choice', async ({ page }) => {
    await reachRuntimeChoice(page, 'pw-onboarding-gate-custom');

    await page.getByTestId('onboarding-runtime-choice-custom').click();

    await expect(page.getByTestId('onboarding-runtime-choice-custom')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(nextButton(page)).toBeEnabled();
    await expect(nextButton(page)).toContainText(/custom/i);
  });

  test('the two runtime options are mutually exclusive', async ({ page }) => {
    // Two options both reading `aria-pressed="true"` would leave the user
    // unable to tell what they are about to configure.
    await reachRuntimeChoice(page, 'pw-onboarding-gate-exclusive');

    await page.getByTestId('onboarding-runtime-choice-cloud').click();
    await expect(page.getByTestId('onboarding-runtime-choice-cloud')).toHaveAttribute(
      'aria-pressed',
      'true'
    );

    await page.getByTestId('onboarding-runtime-choice-custom').click();

    await expect(page.getByTestId('onboarding-runtime-choice-custom')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(page.getByTestId('onboarding-runtime-choice-cloud')).toHaveAttribute(
      'aria-pressed',
      'false'
    );
  });

  test('onboarding is not marked complete while the flow is still gated', async ({ page }) => {
    // The gate is only meaningful if the user is genuinely still in onboarding.
    // If `onboarding_completed` were already true, a reload would drop them
    // into the app and the disabled button would be protecting nothing.
    await reachRuntimeChoice(page, 'pw-onboarding-gate-incomplete');

    const completed = await callCoreRpc<boolean | { result?: boolean }>(
      'openhuman.config_get_onboarding_completed',
      {}
    );
    // `Boolean(completed?.result)` alone would turn a malformed response — one
    // with no `result` at all — into `false` and quietly satisfy the assertion.
    // Require an actual boolean first, so a shape change fails loudly here
    // instead of being read as "onboarding is incomplete".
    const value = typeof completed === 'boolean' ? completed : completed?.result;
    expect(typeof value).toBe('boolean');
    expect(value).toBe(false);
  });
});
