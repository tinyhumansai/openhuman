import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Token & Cost panel — what the compression switches do when their settings
 * never arrive (#5925).
 *
 * Two separate properties, and before #5925 the panel got both wrong:
 *
 * 1. **A settings failure must disable the switches.** `settings` is `null`, so
 *    every switch fell back to `checked={settings?.x ?? false}` and rendered
 *    unchecked but *live*. Toggling one sent a `tokenjuice_settings_update`
 *    built on nothing, silently writing `false` over whatever the user had.
 * 2. **A savings failure must NOT disable them.** The old loader awaited both
 *    calls in one `Promise.all`, so a failure in the display-only savings
 *    figure threw away the settings too and took the whole configuration
 *    surface down with it. The two are now loaded independently.
 *
 * No spec in any lane reached this panel before. The jsdom suite added with
 * #5925 covers the same ground with a mocked transport; this drives the real
 * panel in a browser with only the failing RPC stubbed.
 */

const USAGE_TAB = '/#/connections?tab=usage#tokens';

/** English labels — the browser lane has i18n loaded, so `t()` has resolved. */
const COMPRESSION_SWITCH = 'Enable compression';
const SEARCH_SWITCH = 'Search results';

/**
 * Fail exactly one RPC method, passing every other call through to the real
 * core, so only the property under test is perturbed.
 */
async function failMethod(page: Page, method: string, message: string) {
  await page.route('**/rpc', async (route, request) => {
    const body = JSON.parse(request.postData() || '{}');
    if (body.method === method) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code: -32000, message } }),
      });
      return;
    }
    await route.continue();
  });
}

async function openUsageTab(page: Page) {
  await page.goto(USAGE_TAB);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Token & Cost — settings that fail to load', () => {
  test('a settings failure disables every compression switch', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w6-tokenusage-settings-fail', '/connections?tab=usage');
    await failMethod(
      page,
      'openhuman.tokenjuice_settings_get',
      'the core could not read compression settings'
    );
    await openUsageTab(page);

    const compression = page.getByRole('switch', { name: COMPRESSION_SWITCH });
    await expect(compression).toBeVisible({ timeout: 30_000 });

    // The assertion #5925 added. Without `disabled={settings === null}` the
    // switch renders unchecked and fully interactive, and clicking it patches
    // the backend from a settings object that was never loaded.
    await expect(compression).toBeDisabled();
    await expect(page.getByRole('switch', { name: SEARCH_SWITCH })).toBeDisabled();

    // Every switch on the panel, not just the two named above: a partial fix
    // that missed one would leave exactly the hazard this closes.
    const switches = page.getByRole('switch');
    const count = await switches.count();
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < count; i += 1) {
      await expect(switches.nth(i)).toBeDisabled();
    }
  });

  test('a savings failure leaves the compression switches usable', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w6-tokenusage-savings-fail', '/connections?tab=usage');
    await failMethod(
      page,
      'openhuman.tokenjuice_savings_stats',
      'the core could not read savings statistics'
    );
    await openUsageTab(page);

    const compression = page.getByRole('switch', { name: COMPRESSION_SWITCH });
    await expect(compression).toBeVisible({ timeout: 30_000 });

    // The half the old `Promise.all` broke: settings loaded fine, so the
    // configuration controls must stay interactive even though the savings
    // figure beside them could not be fetched.
    await expect(compression).toBeEnabled();
    await expect(page.getByRole('switch', { name: SEARCH_SWITCH })).toBeEnabled();
  });
});
