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
 * Data Management E2E spec (ID 13.5).
 * Covers:
 * - 13.5.1 Clear App Data confirmation
 * - 13.5.2 Cache Reset (via Clear App Data flow)
 * - 13.5.3 Full State Reset
 *
 * Uses isolated OPENHUMAN_WORKSPACE (handled by e2e-run-spec.sh).
 */

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SettingsDataMgmtE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SettingsDataMgmtE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Settings - Data Management', () => {
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
    await triggerAuthDeepLinkBypass('e2e-data-mgmt');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SettingsDataMgmtE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('shows Clear App Data confirmation dialog and handles Cancel (13.5.1)', async () => {
    stepLog('navigating to /settings');
    await navigateViaHash('/settings');

    await waitForText('Clear App Data', 15_000);

    stepLog('clicking Clear App Data');
    await clickText('Clear App Data');

    await waitForText('This will sign you out and permanently delete local app data', 5_000);

    stepLog('clicking Cancel');
    await clickText('Cancel');

    // Confirm dialog is gone and we are still in settings
    expect(await textExists('This will sign you out and permanently delete local app data')).toBe(
      false
    );
    expect(await textExists('Clear App Data')).toBe(true);
  });

  it('performs Full State Reset (13.5.3)', async () => {
    // We already confirmed the Cancel flow above.
    // Now we confirm the actual reset.
    stepLog('navigating to /settings (reset flow)');
    await navigateViaHash('/settings');
    await waitForText('Clear App Data', 15_000);

    stepLog('opening reset modal');
    await clickText('Clear App Data');
    await waitForText('This will sign you out', 5_000);

    stepLog('clicking confirm Clear App Data');
    // The button text in the modal is also "Clear App Data".
    // clickText clicks the first one it finds.
    await clickText('Clear App Data');

    // After reset, the app should restart and show the Welcome screen.
    // In E2E tests, the restartApp command might just close the window or
    // the mock server might capture a request.
    // However, the test runner handles the process lifecycle.

    // We expect to land back on the login/welcome screen
    await waitForText('Welcome', 25_000);
    expect(await textExists('Sign in')).toBe(true);
  });
});
