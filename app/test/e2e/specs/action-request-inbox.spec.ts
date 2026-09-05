// @ts-nocheck
/**
 * ActionRequest inbox E2E (M1.2.3 / #18) — bounded user-visible route smoke.
 *
 * Full approve/reject + storage fail-closed + Core refresh + pending-filter
 * behavior is covered by the Vitest UI integration suite
 * (`app/src/pages/ActionRequestInbox.bridge.test.tsx`) against a mocked
 * client interface. This Appium spec verifies the protected route mounts the
 * inbox shell end-to-end through Tauri/CEF.
 *
 * Decision mutations against a live Core ActionRequest catalog are not
 * exercised here (require configured YOUPET_TENANT_ID + Core fixtures);
 * empty/error render states are both acceptable for route-level smoke.
 */
import { waitForApp } from '../helpers/app-helpers';
import { waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-action-request-inbox';

describe('ActionRequest inbox route', function () {
  // WDIO captures the Mocha runnable timeout before entering wrapped hooks,
  // so this budget must be set at suite definition time rather than inside
  // the `before` callback.
  this.timeout(90_000);

  before(async function beforeSuite() {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('mounts the Action Request Inbox shell on /action-requests', async function () {
    this.timeout(90_000);
    await navigateViaHash('/action-requests');

    await waitForText('Action Request Inbox', 20_000);

    const mounted = await browser.execute(() => {
      return Boolean(document.querySelector('[data-testid="action-request-inbox"]'));
    });
    expect(mounted).toBe(true);

    // Either list content, empty state, loading, or a structured error is fine —
    // the page must not be a blank shell.
    const hasOperatorSurface = await browser.execute(() => {
      return Boolean(
        document.querySelector('[data-testid="action-request-list"]') ||
        document.querySelector('[data-testid="action-request-empty"]') ||
        document.querySelector('[data-testid="action-request-loading"]') ||
        document.querySelector('[data-testid="action-request-error"]')
      );
    });
    expect(hasOperatorSurface).toBe(true);
  });
});
