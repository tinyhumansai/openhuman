/**
 * Chat composer — the model chip opens the provider/model picker, and the
 * choice sticks for the next turn.
 *
 * Zero browser coverage today: no spec in `app/test/playwright/specs` mentions
 * the model selector at all. The chip is the only text-labelled control in the
 * composer footer, and it is what tells a user which model their next message
 * will go to — so a picker that opens but does not change the label, or a label
 * that resets on the next send, is a silent correctness problem.
 *
 * `ChatComposer.tsx:453` calls it a "Read-only model chip". It is not: it
 * renders a `Button` that opens `ProviderModelPickerDialog`
 * (`ModelQualityPill.tsx:104-121`), disabled only when `!onValueChange ||
 * loading`. The comment is stale; the control is live.
 *
 * # Environment dependency, stated plainly
 *
 * Which providers appear in the picker comes from core config, and the e2e
 * fixture does not seed a cloud provider. The always-available option is the
 * managed tier ("Managed by OpenHuman"), which the dialog deliberately lets you
 * pick with no model id (`ProviderModelPickerDialog.tsx:189-196`). Where a case
 * needs a provider this fixture does not have, it skips with a reason rather
 * than asserting something weaker — per the lane rules, a case that cannot be
 * driven in the browser is skipped, not quietly downgraded.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-model-override';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

async function openChat(page: Page): Promise<void> {
  // `bootAuthenticatedPage(.., '/chat')` already navigates to #/chat and runs
  // `waitForAppReady`. The sibling chat specs then repeat both; doing so races
  // the app's own boot and, under a loaded machine, left the first tests of a
  // file staring at a blank #root until the 60s test timeout. One navigation.
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

/**
 * The model chip, by its analytics id rather than its accessible name.
 * `getByRole('button', { name: 'Model' })` is ambiguous once a thread exists:
 * sidebar thread rows are also `role="button"`, and a thread titled from a
 * prompt about models matches the same name — a strict-mode violation that only
 * appears after the first turn is sent.
 */
const modelChip = (page: Page): Locator =>
  page.locator('[data-analytics-id="chat-model-selector"]');
const pickerTitle = (page: Page): Locator => page.getByText('Choose provider and model');

/**
 * These are browser specs against a freshly built bundle, and the first few to
 * run pay the app's cold start: a fresh `dist-web` plus a just-rebuilt core
 * means first paint can take most of a minute, while every subsequent test in
 * the same session settles at ~1s.
 *
 * Measured on this suite: cases 1-4 of the first spec failed at ~60s with a
 * blank `#root`, case 5 of the SAME file passed at 25.3s, and all 13 cases
 * after it passed in ~1s. Nothing about the app was wrong — the per-test budget
 * (60s locally, `playwright.config.ts:10`) was simply consumed by warm-up.
 *
 * Raising the budget for this describe rather than editing the shared config:
 * it is a statement about these tests, it masks nothing (the assertions are
 * unchanged and a genuinely broken app still fails), and whichever spec happens
 * to sort first should not be the one that flakes.
 */
test.describe.configure({ timeout: 120_000 });

test.describe('Chat composer model override', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([{ content: 'Reply after the model override.' }])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '5');
  });

  test('the model chip is an interactive control that opens the picker', async ({ page }) => {
    await openChat(page);

    const chip = modelChip(page);
    await expect(chip).toBeVisible();
    await expect(
      chip,
      'the chip is enabled whenever an onValueChange handler is wired and the catalog is loaded'
    ).toBeEnabled();

    await chip.click();
    await expect(pickerTitle(page)).toBeVisible({ timeout: 10_000 });
  });

  test('cancelling the picker leaves the current model untouched', async ({ page }) => {
    await openChat(page);

    const chip = modelChip(page);
    const before = (await chip.textContent())?.trim() ?? '';
    // Without this the case is vacuous: if the chip rendered no label, `before`
    // and the post-cancel read are both '' and the comparison passes while
    // proving nothing.
    expect(before, 'the model chip must name the current model').not.toBe('');

    await chip.click();
    await expect(pickerTitle(page)).toBeVisible({ timeout: 10_000 });
    await page.getByRole('button', { name: 'Cancel' }).click();

    await expect(pickerTitle(page)).toHaveCount(0);
    expect((await chip.textContent())?.trim() ?? '').toBe(before);
  });

  /*
   * REMOVED — 'choosing a provider updates the chip label and survives the next
   * turn'.
   *
   * It passed with the pill's `onValueChange` replaced by a no-op, which means
   * it never verified that choosing anything took effect. Reading it back, the
   * assertions were: capture the label AFTER selecting, assert it is non-empty,
   * then assert it is unchanged by a turn. Nothing compared it to the label
   * BEFORE, so a handler that discards the selection satisfies every line.
   *
   * The obvious repair — assert the label CHANGED — cannot be made honest with
   * this fixture. The only selectable provider here is the managed tier
   * (`ProviderModelPickerDialog.tsx:189-196` lets it be picked with no model
   * id), and that is already the active model, so a correct selection is
   * legitimately a no-op with nothing observable to assert. Driving a real
   * change needs a second, seeded cloud provider, which the e2e core config
   * does not create.
   *
   * Deleted rather than weakened or left green: a test that cannot distinguish
   * a working picker from a discarded selection is worse than no test, because
   * it reports coverage of exactly the behaviour it fails to check.
   */
});
