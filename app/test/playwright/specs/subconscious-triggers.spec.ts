import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const USER_ID = 'pw-subconscious-triggers';
const BRAIN_SUBCONSCIOUS = '/brain?tab=subconscious';

/** Reset the trigger pipeline to a known disabled baseline (config-driven). */
async function resetTriggersDisabled(): Promise<void> {
  await callCoreRpc('openhuman.heartbeat_settings_set', {
    triggers_enabled: false,
    subconscious_mode: 'off',
    max_promotions_per_hour: 30,
  });
}

async function openPanel(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, BRAIN_SUBCONSCIOUS);
  await waitForAppReady(page);
  await expect(page.getByTestId('subconscious-triggers-panel')).toBeVisible({ timeout: 20_000 });
  // Status resolves once the first poll returns.
  await expect(page.getByTestId('subconscious-triggers-status')).toBeVisible({ timeout: 20_000 });
}

test.describe('Brain — Subconscious Triggers panel', () => {
  test.beforeEach(async () => {
    await resetTriggersDisabled();
  });

  test('renders the status rows with the disabled baseline', async ({ page }) => {
    await openPanel(page);

    // Pipeline shows disabled + the activation hint, orchestrator stopped-by-config.
    await expect(page.getByTestId('row-pipeline')).toContainText(/Disabled/i);
    await expect(page.getByTestId('subconscious-triggers-disabled-hint')).toBeVisible();

    // Structural rows are present.
    await expect(page.getByTestId('row-mode')).toBeVisible();
    await expect(page.getByTestId('row-promotions')).toContainText('30');
    await expect(page.getByTestId('row-queue')).toBeVisible();
    // Reserved thread ids surface verbatim from the core.
    await expect(page.getByTestId('row-orchestrator-thread')).toContainText(
      'subconscious:orchestrator'
    );
    await expect(page.getByTestId('row-user-thread')).toContainText('subconscious:user');
  });

  test('enabling the pipeline flips it to event-driven and starts the orchestrator', async ({
    page,
  }) => {
    await openPanel(page);
    await expect(page.getByTestId('row-pipeline')).toContainText(/Disabled/i);

    // Toggle on → core enables triggers + event_driven mode and bootstraps.
    await page.getByTestId('subconscious-triggers-toggle').click();

    await expect(page.getByTestId('row-pipeline')).toContainText(/Enabled/i, { timeout: 20_000 });
    await expect(page.getByTestId('row-mode')).toContainText('event_driven');
    await expect(page.getByTestId('row-orchestrator')).toContainText(/Running/i, {
      timeout: 20_000,
    });
    // Once running, the queue depth is a number (0 when idle), not the em-dash.
    await expect(page.getByTestId('row-queue')).not.toContainText('—');
    // The toggle now offers the inverse action and the hint is gone.
    await expect(page.getByTestId('subconscious-triggers-toggle')).toContainText(/Disable/i);
    await expect(page.getByTestId('subconscious-triggers-disabled-hint')).toHaveCount(0);
  });

  test('disabling the pipeline returns it to disabled', async ({ page }) => {
    await openPanel(page);

    // Enable first.
    await page.getByTestId('subconscious-triggers-toggle').click();
    await expect(page.getByTestId('row-pipeline')).toContainText(/Enabled/i, { timeout: 20_000 });

    // Then disable.
    await page.getByTestId('subconscious-triggers-toggle').click();
    await expect(page.getByTestId('row-pipeline')).toContainText(/Disabled/i, { timeout: 20_000 });
    await expect(page.getByTestId('subconscious-triggers-toggle')).toContainText(/Enable/i);
  });

  test('refresh re-fetches without error', async ({ page }) => {
    await openPanel(page);

    await page.getByTestId('subconscious-triggers-refresh').click();

    // Still showing status, no error surfaced.
    await expect(page.getByTestId('subconscious-triggers-status')).toBeVisible();
    await expect(page.getByTestId('subconscious-triggers-error')).toHaveCount(0);
    await expect(page.getByTestId('row-pipeline')).toBeVisible();
  });
});
