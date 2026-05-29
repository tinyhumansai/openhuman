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

test.describe('Settings - Account Preferences', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-account-user');
    await emulateTauriRuntime(page);
  });

  test('renders the account settings section route', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/account');

    await expect(page.getByRole('heading', { name: 'Account' })).toBeVisible();
    await expect(page.getByTestId('settings-nav-recovery-phrase')).toBeVisible();
    await expect(page.getByTestId('settings-nav-team')).toBeVisible();
    await expect(page.getByTestId('settings-nav-privacy')).toBeVisible();
  });

  test('saves a generated recovery phrase and exposes configured wallet state', async ({
    page,
  }) => {
    await gotoSettingsRoute(page, '/settings/recovery-phrase');

    await expect(page.getByRole('button', { name: 'Copy to Clipboard' })).toBeVisible();
    await page.locator('input[type="checkbox"]').first().check();
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

    await expect(page.getByRole('heading', { name: 'Privacy & Security' })).toBeVisible();
    await expect(page.getByText('Share Anonymized Usage Data')).toBeVisible();

    await page.getByTestId('privacy-analytics-toggle').click();
    await page.getByTestId('privacy-meet-handoff-toggle').click();

    await expect
      .poll(async () => {
        const analytics = await callCoreRpc<{ result?: { enabled?: boolean } }>(
          'openhuman.config_get_analytics_settings',
          {}
        );
        const meet = await callCoreRpc<{ result?: { auto_orchestrator_handoff?: boolean } }>(
          'openhuman.config_get_meet_settings',
          {}
        );
        return {
          analyticsEnabled: Boolean(analytics.result?.enabled),
          meetHandoff: Boolean(meet.result?.auto_orchestrator_handoff),
        };
      })
      .toEqual({ analyticsEnabled: !initialAnalytics, meetHandoff: !initialMeet });

    const snapshot = await callCoreRpc<{
      result?: { analyticsEnabled?: boolean; meetAutoOrchestratorHandoff?: boolean };
    }>('openhuman.app_state_snapshot', {});
    expect(Boolean(snapshot.result?.analyticsEnabled)).toBe(!initialAnalytics);
    expect(Boolean(snapshot.result?.meetAutoOrchestratorHandoff)).toBe(!initialMeet);
  });

  test('opens the billing route and settles the redirect status copy', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/billing');

    await expect(page.getByRole('heading', { name: 'Open billing dashboard' })).toBeVisible();
    await expect(
      page.getByText(
        /If your browser did not open, use the button above\.|The browser could not be opened automatically\.|Opening your browser\.\.\./
      )
    ).toBeVisible();

    await page.getByRole('button', { name: 'Back to settings' }).click();
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toContain('/settings');
  });
});
