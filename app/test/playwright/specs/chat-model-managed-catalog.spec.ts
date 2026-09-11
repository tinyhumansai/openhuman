/**
 * Managed OpenRouter catalog in the composer's model picker.
 *
 * `#6201` (feat(inference): surface the managed OpenRouter catalog in the model
 * picker) shipped with unit tests only, and those tests mock
 * `listProviderModels` outright — so nothing exercised the chain that actually
 * has to work: browser → core RPC `openhuman.inference_list_models` →
 * `list_configured_models_from_config` resolving the managed backend's URL and
 * session JWT → `GET /openai/v1/models?catalog=openrouter` → back through the
 * picker's render and round-trip helpers.
 *
 * # Why the sibling spec could not cover this
 *
 * `chat-model-override.spec.ts` documents (and deletes) a case that could not
 * be made honest: the only selectable source in this fixture was the managed
 * tier, managed was already the active model, and managed carried no model id —
 * so "select managed" was a legitimate no-op with nothing observable to assert.
 *
 * #6201 is exactly what changes that. Managed now carries a model id, so a
 * selection is observable in the chip label, and the same source can be driven
 * to two distinguishable states. That is what makes these cases possible at all.
 *
 * # What is pinned here, and why each one can regress
 *
 * 1. The managed pane lists catalog models. Before #6201 it showed prose and no
 *    models; a regression in the fetch seam (`fetchSlug`, the managed URL, the
 *    session credential) puts it straight back there.
 * 2. Round-trip. A pinned managed id is encoded BARE, and `selectionFromValue`
 *    used to decode anything without a `:` to null — the selection vanished on
 *    reopen. Only a reopen catches it.
 * 3. A `:free` variant renders its full name. `displayValue` used to split on
 *    `:` as if it were `providerSlug:model` and render the chip as "free".
 * 4. Options are labelled by display name and charged price, not a bare slug.
 *
 * Item 4 of the PR — cost classification of `openrouter/<a>/<b>` as `Managed`
 * rather than `Byok` — is deliberately NOT here. It is a pure core-side routing
 * decision with no browser-observable surface, and `route_tests.rs` covers it
 * directly.
 *
 * # Environment dependency, stated plainly
 *
 * The catalog comes from the mock backend's `GET /openai/v1/models` route
 * (`scripts/mock-api/routes/llm.mjs`), which serves the curated tier list
 * without `?catalog=openrouter` and the passthrough catalog with it. That
 * two-shape distinction is the feature. Against a real backend with
 * `OPENROUTER_PASSTHROUGH_ENABLED` off the catalog is empty and no select
 * renders — expected behaviour, pinned by its own case below.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-managed-catalog';

/** Mirrors the fixture in `scripts/mock-api/routes/llm.mjs`. */
const PLAIN_ID = 'openrouter/nex-agi/nex-n2.5-mini';
const FREE_ID = 'openrouter/nex-agi/nex-n2.5-mini:free';
const NO_METADATA_ID = 'openrouter/acme/plain-model';

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

/**
 * Make `isTauri()` true for this page.
 *
 * `listProviderModels` (`aiSettingsApi.ts:778-781`) returns `[]` without
 * issuing any RPC when `isTauri()` is false — its comment says "browser dev
 * mode has no RPC bridge", which is stale for this lane: `callCoreRpc` reaches
 * the core over plain HTTP here and ~58 other RPCs succeed on a single page
 * load. Without this shim the managed catalog fetch never leaves the browser,
 * the pane renders no select, and every case below fails for a reason that has
 * nothing to do with #6201.
 *
 * The stubbed `invoke` is never reached: `callCoreRpc` dispatches
 * `openhuman.*` methods over HTTP, not through the Tauri IPC bridge. Same
 * pattern as `settings-advanced-config.spec.ts` and
 * `settings-recovery-phrase-replace.spec.ts`.
 */
async function emulateTauriRuntime(page: Page): Promise<void> {
  await page.evaluate(() => {
    const win = window as typeof window & {
      isTauri?: boolean;
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    win.isTauri = true;
    win.__TAURI_INTERNALS__ = win.__TAURI_INTERNALS__ ?? {};
    win.__TAURI_INTERNALS__.invoke = win.__TAURI_INTERNALS__.invoke ?? (async () => null);
  });
}

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
  await emulateTauriRuntime(page);
}

const modelChip = (page: Page): Locator =>
  page.locator('[data-analytics-id="chat-model-selector"]');
const pickerTitle = (page: Page): Locator => page.getByText('Choose provider and model');
const managedSelect = (page: Page): Locator => page.getByTestId('model-picker-managed-select');

/**
 * Open the picker and land on the managed source's pane.
 *
 * The source row is clicked ONLY when the dialog did not already open on the
 * managed pane. `selectSource` (`ProviderModelPickerDialog.tsx:211-213`) does an
 * unconditional `setModel('')`, so re-clicking the source you are already on
 * silently discards the pinned model — which would destroy exactly the state a
 * reopen is meant to verify and turn a passing round-trip into a false failure.
 * When a managed model is pinned the dialog opens on managed anyway, because
 * `source` is seeded from `initial?.source`.
 */
async function openManagedPane(page: Page): Promise<void> {
  await modelChip(page).click();
  await expect(pickerTitle(page)).toBeVisible({ timeout: 10_000 });
  if ((await page.getByTestId('model-picker-managed-pane').count()) === 0) {
    await page.getByText('Managed by OpenHuman', { exact: true }).first().click();
  }
  await expect(page.getByTestId('model-picker-managed-pane')).toBeVisible();
}

/** Cold start budget, as measured and explained in `chat-model-override.spec.ts`. */
test.describe.configure({ timeout: 120_000 });

test.describe('Managed OpenRouter catalog in the model picker', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  test('the managed pane lists the backend catalog, labelled by name and price', async ({
    page,
  }) => {
    await openChat(page);
    await openManagedPane(page);

    const select = managedSelect(page);
    await expect(
      select,
      'before #6201 the managed pane rendered prose and no model control at all'
    ).toBeVisible({ timeout: 20_000 });

    // The catalog reached the browser through the real core fetch, not a stub.
    const optionValues = await select
      .locator('option')
      .evaluateAll(nodes => nodes.map(node => (node as HTMLOptionElement).value));
    expect(optionValues).toContain(PLAIN_ID);
    expect(optionValues).toContain(FREE_ID);
    // "Automatic" is always present so a pin is reversible.
    expect(optionValues).toContain('');

    // Labelled by display name + charged price, not the bare slug. Asserting
    // the price text matters: the id alone would satisfy a label that ignored
    // `display_name` / `pricing` entirely.
    const plainLabel = await select.locator(`option[value="${PLAIN_ID}"]`).textContent();
    expect(plainLabel?.trim()).toBe('Nex N2.5 Mini — $0.15/$0.6 per 1M');

    // A catalog entry with neither name nor pricing falls back to the bare id
    // rather than rendering "undefined" or an empty option.
    const plainestLabel = await select.locator(`option[value="${NO_METADATA_ID}"]`).textContent();
    expect(plainestLabel?.trim()).toBe(NO_METADATA_ID);
  });

  test('a pinned managed model survives closing and reopening the picker', async ({ page }) => {
    await openChat(page);

    const chip = modelChip(page);
    const before = (await chip.textContent())?.trim() ?? '';
    // Without this the case is vacuous: an empty label before and after would
    // compare equal while proving nothing.
    expect(before, 'the model chip must name the current model').not.toBe('');

    await openManagedPane(page);
    await expect(managedSelect(page)).toBeVisible({ timeout: 20_000 });
    await managedSelect(page).selectOption(PLAIN_ID);
    await page.getByRole('button', { name: 'Use this model' }).click();
    await expect(pickerTitle(page)).toHaveCount(0);

    // The pin took effect: the chip names the pinned model, not what it named
    // before. A handler that discarded the selection fails right here.
    await expect(chip).toHaveText(/nex-n2\.5-mini/, { timeout: 10_000 });
    expect((await chip.textContent())?.trim() ?? '').not.toBe(before);

    // The round trip: reopen and the select still shows the pinned id.
    // `selectionFromValue` decoded a bare id (no `:`) to null, so the selection
    // was silently dropped here and the pane reopened on "Automatic".
    await openManagedPane(page);
    await expect(managedSelect(page)).toBeVisible({ timeout: 20_000 });
    await expect(
      managedSelect(page),
      'a pinned managed id round-trips through selectionValue/selectionFromValue'
    ).toHaveValue(PLAIN_ID);
  });

  test('a :free variant keeps its full name on the chip', async ({ page }) => {
    await openChat(page);
    await openManagedPane(page);
    await expect(managedSelect(page)).toBeVisible({ timeout: 20_000 });

    await managedSelect(page).selectOption(FREE_ID);
    await page.getByRole('button', { name: 'Use this model' }).click();
    await expect(pickerTitle(page)).toHaveCount(0);

    const chip = modelChip(page);
    // `displayValue` split on the FIRST `:` as if this were `providerSlug:model`,
    // reducing the whole id to the bare word "free".
    await expect(chip).toHaveText(/nex-n2\.5-mini:free/, { timeout: 10_000 });
    expect(
      (await chip.textContent())?.trim(),
      'the chip must not collapse the variant suffix into just "free"'
    ).not.toBe('free');

    // And it round-trips: the variant suffix is not lost on reopen either.
    await openManagedPane(page);
    await expect(managedSelect(page)).toHaveValue(FREE_ID, { timeout: 20_000 });
  });

  test('a pinned model can be cleared back to automatic', async ({ page }) => {
    await openChat(page);
    await openManagedPane(page);
    await expect(managedSelect(page)).toBeVisible({ timeout: 20_000 });
    await managedSelect(page).selectOption(PLAIN_ID);
    await page.getByRole('button', { name: 'Use this model' }).click();
    await expect(pickerTitle(page)).toHaveCount(0);
    const pinned = (await modelChip(page).textContent())?.trim() ?? '';
    expect(pinned).toMatch(/nex-n2\.5-mini/);

    await openManagedPane(page);
    await expect(managedSelect(page)).toHaveValue(PLAIN_ID, { timeout: 20_000 });
    await managedSelect(page).selectOption('');
    await page.getByRole('button', { name: 'Use this model' }).click();
    await expect(pickerTitle(page)).toHaveCount(0);

    // Back to automatic routing: the chip no longer names the pinned model.
    await expect(modelChip(page)).not.toHaveText(/nex-n2\.5-mini/, { timeout: 10_000 });
  });

  test('an empty catalog renders no select, exactly as before the feature', async ({ page }) => {
    // Models the real backend with OPENROUTER_PASSTHROUGH_ENABLED off: it
    // returns an empty set with HTTP 200, not an error. The managed pane must
    // degrade to its pre-#6201 appearance rather than surfacing a failure.
    await setMockBehavior('managedCatalogEmpty', 'true');
    await openChat(page);
    await openManagedPane(page);

    await expect(page.getByTestId('model-picker-managed-pane')).toBeVisible();
    await expect(managedSelect(page)).toHaveCount(0);
    // Managed is still selectable with no model id — the original contract.
    await page.getByRole('button', { name: 'Use this model' }).click();
    await expect(pickerTitle(page)).toHaveCount(0);
  });
});
