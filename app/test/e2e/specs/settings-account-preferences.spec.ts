// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { clickSelector, clickText, textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-account-preferences';

async function waitForHashContains(fragment: string, timeout = 10_000): Promise<void> {
  await browser.waitUntil(
    async () => String(await browser.execute(() => window.location.hash)).includes(fragment),
    { timeout, interval: 250, timeoutMsg: `hash did not include ${fragment}` }
  );
}

describe('Settings - Account Preferences', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('renders the account settings section route', async () => {
    await navigateViaHash('/settings/account');

    await waitForText('Account', 15_000);
    await waitForText('Recovery Phrase', 15_000);
    await waitForText('Connections', 15_000);
    await waitForText('Privacy', 15_000);
  });

  it('saves a generated recovery phrase and exposes configured wallet state', async () => {
    await navigateViaHash('/settings/recovery-phrase');

    await waitForText('Copy to Clipboard', 15_000);
    await clickSelector('input[type="checkbox"]');
    await clickText('Save Recovery Phrase', 10_000);

    await waitForText('Recovery phrase saved', 20_000);
    await waitForText('Multi-chain wallet identities are ready', 20_000);

    const wallet = await callOpenhumanRpc('openhuman.wallet_status', {});
    expect(wallet.ok).toBe(true);
    expect(wallet.result?.result?.configured).toBe(true);
    expect((wallet.result?.result?.accounts ?? []).length).toBeGreaterThan(0);

    await navigateViaHash('/settings/connections');
    await waitForText('Web3 Wallet', 15_000);
    await waitForText('Configured', 15_000);
  });

  it('persists privacy analytics and meet handoff toggles to core config', async () => {
    const beforeAnalytics = await callOpenhumanRpc('openhuman.config_get_analytics_settings', {});
    const beforeMeet = await callOpenhumanRpc('openhuman.config_get_meet_settings', {});
    expect(beforeAnalytics.ok).toBe(true);
    expect(beforeMeet.ok).toBe(true);

    const initialAnalytics = Boolean(beforeAnalytics.result?.result?.enabled);
    const initialMeet = Boolean(beforeMeet.result?.result?.auto_orchestrator_handoff);

    await navigateViaHash('/settings/privacy');
    await waitForText('Privacy', 15_000);
    await waitForText('Share Anonymized Usage Data', 15_000);

    await clickSelector('[data-testid="privacy-analytics-toggle"]');
    await clickSelector('[data-testid="privacy-meet-handoff-toggle"]');

    await browser.waitUntil(
      async () => {
        const analytics = await callOpenhumanRpc('openhuman.config_get_analytics_settings', {});
        const meet = await callOpenhumanRpc('openhuman.config_get_meet_settings', {});
        return (
          analytics.ok &&
          meet.ok &&
          Boolean(analytics.result?.result?.enabled) === !initialAnalytics &&
          Boolean(meet.result?.result?.auto_orchestrator_handoff) === !initialMeet
        );
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'privacy settings did not persist' }
    );

    const snapshot = await callOpenhumanRpc('openhuman.app_state_snapshot', {});
    expect(snapshot.ok).toBe(true);
    expect(Boolean(snapshot.result?.result?.analyticsEnabled)).toBe(!initialAnalytics);
    expect(Boolean(snapshot.result?.result?.meetAutoOrchestratorHandoff)).toBe(!initialMeet);
  });

  it('opens the billing route and settles the redirect status copy', async () => {
    await navigateViaHash('/settings/billing');

    await waitForHashContains('/settings/billing');
    await waitForText('Open billing dashboard', 15_000);

    await browser.waitUntil(
      async () =>
        (await textExists('If your browser did not open, use the button above.')) ||
        (await textExists('We could not open your browser automatically.')),
      { timeout: 15_000, interval: 500, timeoutMsg: 'billing redirect status did not settle' }
    );

    await clickText('Back to settings', 10_000);
    await waitForHashContains('/settings');
  });
});
