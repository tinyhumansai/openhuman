import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

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

async function gotoSettingsRoute(page: Page, hash: string): Promise<void> {
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function ensureRecoveryPhraseGenerateMode(page: Page): Promise<void> {
  const copyButton = page.getByRole('button', { name: 'Copy to Clipboard' });
  const replaceButton = page.getByRole('button', { name: 'Replace wallet' });

  await expect
    .poll(
      async () => {
        if (await copyButton.isVisible()) return 'generate';
        if (await replaceButton.isVisible()) return 'configured';
        return 'loading';
      },
      { timeout: 15_000 }
    )
    .not.toBe('loading');

  if (await replaceButton.isVisible()) {
    await replaceButton.click();
    await page.getByRole('button', { name: 'I understand, replace my wallet' }).click();
  }

  await expect(copyButton).toBeVisible();
}

test.describe('Settings - Account Preferences', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-account-user');
    await emulateTauriRuntime(page);
  });

  test('renders the account settings section route', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/account');

    // Panel titles were dropped in the PanelPage migration; assert the panel's
    // stable test id instead of the old heading.
    await expect(page.getByTestId('account-panel')).toBeVisible();
    await expect(page.getByText(/Account|Profile/).first()).toBeVisible();
  });

  test('renders the crypto settings section route with recovery phrase + balances', async ({
    page,
  }) => {
    // /settings/crypto is retired and redirects to Connections → Wallet.
    await gotoSettingsRoute(page, '/settings/crypto');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections?tab=wallet');
    await expect(page.getByTestId('wallet-panel')).toBeVisible();
  });

  test('saves a generated recovery phrase and exposes configured wallet state', async ({
    page,
  }) => {
    await gotoSettingsRoute(page, '/settings/recovery-phrase');

    await ensureRecoveryPhraseGenerateMode(page);
    await page.locator('#mnemonic-confirm-checkbox').check();
    await page.getByRole('button', { name: 'Save Recovery Phrase' }).click();

    await expect(page.getByText('Recovery phrase saved')).toBeVisible();
    await expect(page.getByText(/Multi-chain wallet identities are ready/)).toBeVisible();

    await expect
      .poll(async () => {
        const wallet = await callCoreRpc<{
          result?: { configured?: boolean; accounts?: unknown[] };
        }>('openhuman.wallet_status', {});
        return {
          configured: Boolean(wallet.result?.configured),
          accountCount: wallet.result?.accounts?.length ?? 0,
        };
      })
      .toEqual({ configured: true, accountCount: expect.any(Number) });

    const wallet = await callCoreRpc<{ result?: { configured?: boolean; accounts?: unknown[] } }>(
      'openhuman.wallet_status',
      {}
    );
    expect(wallet.result?.configured).toBe(true);
    expect((wallet.result?.accounts ?? []).length).toBeGreaterThan(0);
  });

  test('persists the privacy analytics toggle to core config', async ({ page }) => {
    const beforeAnalytics = await callCoreRpc<{ result?: { enabled?: boolean } }>(
      'openhuman.config_get_analytics_settings',
      {}
    );
    const initialAnalytics = Boolean(beforeAnalytics.result?.enabled);

    await gotoSettingsRoute(page, '/settings/privacy');

    await expect(page.getByTestId('settings-privacy-panel')).toBeVisible();
    await expect(page.getByText('Share Product Analytics and Diagnostics')).toBeVisible();

    // Toggle + confirm each setting sequentially. Clicking both back-to-back and
    // polling for the combined result is racy: each toggle triggers an async
    // save and panel re-render, so the second click can land before the first
    // settles, dropping one update. Also wait for each switch to reflect the
    // persisted initial state before clicking — the panel can render from a
    // not-yet-synced snapshot, and clicking then computes the wrong new value.
    await expect(page.getByTestId('privacy-analytics-toggle')).toBeChecked({
      checked: initialAnalytics,
    });
    await page.getByTestId('privacy-analytics-toggle').click();
    await expect
      .poll(async () => {
        const analytics = await callCoreRpc<{ result?: { enabled?: boolean } }>(
          'openhuman.config_get_analytics_settings',
          {}
        );
        return Boolean(analytics.result?.enabled);
      })
      .toBe(!initialAnalytics);

    const snapshot = await callCoreRpc<{ result?: { analyticsEnabled?: boolean } }>(
      'openhuman.app_state_snapshot',
      {}
    );
    expect(Boolean(snapshot.result?.analyticsEnabled)).toBe(!initialAnalytics);
  });

  test('opens the billing route and settles the redirect status copy', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/billing');

    await expect(page.getByRole('heading', { name: 'Billing' })).toBeVisible();
    // Billing no longer auto-opens the browser; the panel explains billing
    // moved to the web and offers an explicit open button.
    await expect(
      page.getByText(/Subscription changes, payment methods, credits, and invoices are now managed/)
    ).toBeVisible();

    await page.getByRole('button', { name: 'Back to settings' }).click();
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/settings');
  });
});
