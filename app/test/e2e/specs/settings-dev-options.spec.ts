// @ts-nocheck
/**
 * Settings → Developer Options (capability 13.4).
 *
 * Rewritten to follow the cron-jobs-flow pattern: resetApp() bootstraps
 * fresh-install state, then each test mounts a debug sub-panel and
 * asserts the page's headline structure is present.
 *
 * Covers:
 *   - 13.4.1 Webhooks Debug panel
 *   - 13.4.2 Autocomplete Debug → Live Logs section
 *   - 13.4.3 Memory Debug panel
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-dev-options';

describe('Settings - Developer Options', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('mounts Webhooks Debug panel (13.4.1)', async () => {
    await navigateViaHash('/settings/webhooks-debug');

    await waitForText('Webhooks Debug', 15_000);
    await waitForText('Registered Webhooks', 15_000);
    await waitForText('Captured Requests', 15_000);
    expect(await textExists('Refresh')).toBe(true);
  });

  it('mounts Memory Debug panel (13.4.3)', async () => {
    await navigateViaHash('/settings/memory-debug');

    await waitForText('Memory Debug', 15_000);
    await waitForText('Documents', 15_000);
    await waitForText('Namespaces', 15_000);
    await waitForText('Query & Recall', 15_000);
    await waitForText('Clear Namespace', 15_000);
  });

  it('shows Live Logs in Autocomplete Debug panel (13.4.2)', async () => {
    await navigateViaHash('/settings/autocomplete-debug');

    await waitForText('Autocomplete Debug', 15_000);
    await waitForText('Live Logs', 15_000);

    const logsFound = (await textExists('No logs yet.')) || (await textExists('[runtime]'));
    expect(logsFound).toBe(true);
  });
});
