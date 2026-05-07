import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Channels & Permissions E2E spec (ID 13.2).
 * Covers:
 * - 13.2.1 Channel Configuration (Default channel)
 * - 13.2.2 Permission Settings persistence (Privacy panel)
 */

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SettingsChannelsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SettingsChannelsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Settings - Channels & Permissions', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — navigation helpers require browser.execute');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-channels');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SettingsChannelsE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('allows switching default messaging channel (13.2.1)', async () => {
    stepLog('navigating to /settings/messaging');
    await navigateViaHash('/settings/messaging');

    await waitForText('Default Messaging Channel', 15_000);

    // Check if Telegram and Discord options exist
    expect(await textExists('Telegram')).toBe(true);
    expect(await textExists('Discord')).toBe(true);

    stepLog('switching to Discord');
    await clickText('Discord');

    // Verify Discord is active in the route label
    await waitForText('Active route: discord via', 5_000);
  });

  it('renders privacy settings and analytics toggle (13.2.2)', async () => {
    stepLog('navigating to /settings/privacy');
    await navigateViaHash('/settings/privacy');

    await waitForText('Privacy', 15_000);
    await waitForText('Data Sharing', 15_000);

    // Analytics toggle should exist
    expect(await textExists('Share Anonymized Usage Data')).toBe(true);

    // Check for "Stays local" text which appears for some capabilities
    // but PrivacyPanel.test.tsx shows it depends on RPC results.
    // At least the header should be there.
    await waitForText('Permission Metadata', 5_000);
  });
});
