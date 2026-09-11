import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * The embeddings setup modal's "Test connection" affordance, driven in a real
 * browser (PR #5943).
 *
 * `setupTest` calls `openhuman.embeddings_test_connection` with only
 * `{ provider, model, dimensions }` — there is no parameter for an endpoint URL
 * — so a custom OpenAI-compatible provider has nothing to test against. Before
 * #5943 the button stayed enabled and its handler opened with `if (!isCustom)`,
 * so a click produced no request, no result and no error: a control that looked
 * live and silently did nothing. The fix disables it for a custom provider and
 * renders the reason as visible text beside it.
 *
 * Why the reason is text and not a `title`: `Button` applies
 * `disabled:pointer-events-none`, so a disabled control cannot be hovered, and
 * a disabled button is out of the tab order — a tooltip would be unreachable by
 * both mouse and keyboard. Asserting the text is on screen is therefore the
 * assertion that matches the fix's own reasoning.
 *
 * `custom` is a real catalog provider
 * (`src/openhuman/inference/embeddings/catalog.rs` — `PROVIDER_CUSTOM`, label
 * "Custom (OpenAI-compatible)"), so it is served by the real core this lane
 * runs against and needs no fixture.
 */

const CUSTOM_LABEL = /Custom \(OpenAI-compatible\)/;
const TEST_CONNECTION = /Test connection/i;

async function openEmbeddingsTab(page: import('@playwright/test').Page, userId: string) {
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    window.location.hash = '/connections?tab=embeddings';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toContain('tab=embeddings');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Embeddings setup — Test connection for a custom endpoint', () => {
  test('is disabled, and says why, for a custom provider', async ({ page }) => {
    await openEmbeddingsTab(page, 'pw-embed-custom');

    // Selecting "Custom" opens the setup modal (EmbeddingsPanel:146-152).
    await page.getByRole('radio').filter({ hasText: CUSTOM_LABEL }).click();

    const testButton = page.getByRole('button', { name: TEST_CONNECTION });
    await expect(testButton).toBeVisible({ timeout: 15_000 });

    // The fix. Before #5943 this was enabled and its click was a no-op.
    await expect(testButton).toBeDisabled();

    // And the user is told why, in text that survives having no pointer and no
    // focus — the whole reason it is not a `title`.
    const reason = page.getByTestId('embeddings-test-unavailable-reason');
    await expect(reason).toBeVisible();
    await expect(reason).toContainText(/not supported yet/i);
  });

  test('a keyed provider keeps a working Test connection button', async ({ page }) => {
    // The contrast that proves the disable is scoped to `isCustom` and is not
    // just "the button is always dead". OpenAI needs a key, so the button is
    // disabled while the key field is empty and enabled once it is filled —
    // and no unavailable-reason text is shown for a non-custom provider.
    await openEmbeddingsTab(page, 'pw-embed-openai');

    // Scoped to an exact label descendant, not `hasText: /OpenAI/`: the radios
    // carry a label AND a description, so a substring match also hits
    // "Custom (OpenAI-compatible)" and `.first()` then depends on catalog
    // order — the test would silently assert against the custom provider,
    // which is the very case the sibling test covers.
    await page
      .getByRole('radio')
      .filter({ has: page.getByText('OpenAI', { exact: true }) })
      .click();

    const testButton = page.getByRole('button', { name: TEST_CONNECTION });
    await expect(testButton).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('embeddings-test-unavailable-reason')).toHaveCount(0);

    // Empty key → disabled by the `!setupKey.trim()` arm, not by `isCustom`.
    await expect(testButton).toBeDisabled();

    // Located by placeholder, not by the `textbox` role: `setupShowKey` starts
    // false so the field renders as `<input type="password">`, which has no
    // implicit textbox role and would never match.
    const keyField = page.getByPlaceholder('Paste your API key…');
    await expect(keyField).toBeVisible();
    await keyField.fill('sk-not-a-real-key-0000000000');
    await expect(testButton).toBeEnabled();
  });
});
