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
    // The Account family surfaces its leaves via the sub-nav pill row above the
    // panel (the two-pane sidebar replaced the old section-hub list).
    await expect(page.getByTestId('settings-subnav-team')).toBeVisible();
    await expect(page.getByTestId('settings-subnav-privacy')).toBeVisible();
    await expect(page.getByTestId('settings-subnav-migration')).toBeVisible();
    // Recovery phrase + wallet balances live under the Wallet family, not Account.
    await expect(page.getByTestId('settings-subnav-recovery-phrase')).toHaveCount(0);
  });

  test('renders the crypto settings section route with recovery phrase + balances', async ({
    page,
  }) => {
    // /settings/crypto is retired and redirects to the Wallet Balances panel,
    // whose sub-nav family surfaces recovery-phrase + wallet-balances.
    await gotoSettingsRoute(page, '/settings/crypto');

    // Panel titles were dropped in the PanelPage migration; the Wallet family is
    // confirmed by its sub-nav leaves below.
    await expect(page.getByTestId('settings-subnav-recovery-phrase')).toBeVisible();
    await expect(page.getByTestId('settings-subnav-wallet-balances')).toBeVisible();
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

  test('persists privacy analytics and meet handoff toggles to core config', async ({ page }) => {
    const beforeAnalytics = await callCoreRpc<{ result?: { enabled?: boolean } }>(
      'openhuman.config_get_analytics_settings',
      {}
    );
    const beforeMeet = await callCoreRpc<{ result?: { auto_orchestrator_handoff?: boolean } }>(
      'openhuman.config_get_meet_settings',
      {}
    );
    const initialAnalytics = Boolean(beforeAnalytics.result?.enabled);
    const initialMeet = Boolean(beforeMeet.result?.auto_orchestrator_handoff);

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

    await expect(page.getByTestId('privacy-meet-handoff-toggle')).toBeChecked({
      checked: initialMeet,
    });
    await page.getByTestId('privacy-meet-handoff-toggle').click();
    await expect
      .poll(async () => {
        const meet = await callCoreRpc<{ result?: { auto_orchestrator_handoff?: boolean } }>(
          'openhuman.config_get_meet_settings',
          {}
        );
        return Boolean(meet.result?.auto_orchestrator_handoff);
      })
      .toBe(!initialMeet);

    const snapshot = await callCoreRpc<{
      result?: { analyticsEnabled?: boolean; meetAutoOrchestratorHandoff?: boolean };
    }>('openhuman.app_state_snapshot', {});
    expect(Boolean(snapshot.result?.analyticsEnabled)).toBe(!initialAnalytics);
    expect(Boolean(snapshot.result?.meetAutoOrchestratorHandoff)).toBe(!initialMeet);
  });

  test('opens the billing route and settles the redirect status copy', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/billing');

    await expect(page.getByRole('heading', { name: 'Open billing dashboard' })).toBeVisible();
    // Billing no longer auto-opens the browser; the panel explains billing
    // moved to the web and offers an explicit open button.
    await expect(
      page.getByText(/Subscription changes, payment methods, credits, and invoices are now managed/)
    ).toBeVisible();

    await page.getByRole('button', { name: 'Back to settings' }).click();
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/settings');
  });
});
