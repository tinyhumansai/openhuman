// @ts-nocheck
/**
 * Settings → Data Management (capability 13.5).
 *
 * Rewritten to follow the cron-jobs-flow pattern. The "Full State Reset"
 * test intentionally runs LAST — it logs the user out, so anything that
 * follows would need its own resetApp() pass. We keep this spec
 * self-contained so the suite ordering doesn't matter.
 *
 * Covers:
 *   - 13.5.1 Clear App Data confirmation dialog + Cancel
 *   - 13.5.3 Full State Reset → back to Welcome screen
 */
import { waitForApp } from '../helpers/app-helpers';
import { clickText, textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-data-mgmt';

describe('Settings - Data Management', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('shows Clear App Data confirmation dialog and handles Cancel (13.5.1)', async () => {
    await navigateViaHash('/settings');
    await waitForText('Clear App Data', 15_000);

    await clickText('Clear App Data');
    await waitForText('This will sign you out and permanently delete local app data', 5_000);

    await clickText('Cancel');
    expect(await textExists('This will sign you out and permanently delete local app data')).toBe(
      false
    );
    expect(await textExists('Clear App Data')).toBe(true);
  });

  it('performs Full State Reset (13.5.3)', async () => {
    await navigateViaHash('/settings');
    await waitForText('Clear App Data', 15_000);

    await clickText('Clear App Data');
    await waitForText('This will sign you out', 5_000);
    // Second click hits the confirm button in the modal (same label).
    await clickText('Clear App Data');

    // After reset the app reloads to the Welcome screen.
    await waitForText('Welcome', 25_000);
    expect(await textExists('Sign in')).toBe(true);
  });
});
