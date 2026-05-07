import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Developer Options E2E spec (ID 13.4).
 * Covers:
 * - 13.4.1 Webhook Inspection
 * - 13.4.2 Runtime Logs (Live Logs in debug panels)
 * - 13.4.3 Memory Debug
 */

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SettingsDevOptionsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SettingsDevOptionsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Settings - Developer Options', () => {
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
    await triggerAuthDeepLinkBypass('e2e-dev-options');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SettingsDevOptionsE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('mounts Webhooks Debug panel (13.4.1)', async () => {
    stepLog('navigating to /settings/webhooks-debug');
    await navigateViaHash('/settings/webhooks-debug');

    await waitForText('Webhooks Debug', 15_000);
    await waitForText('Registered Webhooks', 15_000);
    await waitForText('Captured Requests', 15_000);

    // Check if refresh button exists
    expect(await textExists('Refresh')).toBe(true);
  });

  it('mounts Memory Debug panel (13.4.3)', async () => {
    stepLog('navigating to /settings/memory-debug');
    await navigateViaHash('/settings/memory-debug');

    await waitForText('Memory Debug', 15_000);
    await waitForText('Documents', 15_000);
    await waitForText('Namespaces', 15_000);
    await waitForText('Query & Recall', 15_000);
    await waitForText('Clear Namespace', 15_000);
  });

  it('shows Live Logs in Autocomplete Debug panel (13.4.2)', async () => {
    stepLog('navigating to /settings/autocomplete-debug');
    await navigateViaHash('/settings/autocomplete-debug');

    await waitForText('Autocomplete Debug', 15_000);
    await waitForText('Live Logs', 15_000);

    // Confirm "No logs yet." or actual logs are visible
    const logsFound = (await textExists('No logs yet.')) || (await textExists('[runtime]'));
    expect(logsFound).toBe(true);
  });
});
