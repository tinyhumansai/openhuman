// @ts-nocheck
/**
 * Settings → Developer Options (capability 13.4).
 *
 * Rewritten to follow the cron-jobs-flow pattern: resetApp() bootstraps
 * fresh-install state, then each test mounts a debug sub-panel and
 * asserts the page's headline structure is present.
 *
 * Covers the retained Memory Debug panel.
 */
import { waitForApp } from '../helpers/app-helpers';
import { waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-dev-options';

describe('Settings - Developer Options', function () {
  this.timeout(90_000);

  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('mounts Memory Debug panel (13.4.3)', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/memory-debug');

    await waitForText('Documents', 15_000);
    await waitForText('Namespaces', 15_000);
    await waitForText('Query & Recall', 15_000);
    await waitForText('Clear Namespace', 15_000);
  });
});
