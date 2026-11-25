/** Approval history settings screen against the mocked desktop core. */
import { waitForApp } from '../helpers/app-helpers';
import { waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-security-approval-history';

describe('security approval history', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens the approval history and displays its rows or empty state', async () => {
    await navigateViaHash('/settings/approval-history');
    await waitForTestId('approval-history-panel', 15_000);
    await browser.waitUntil(
      async () =>
        (await browser.execute(() =>
          Boolean(
            document.querySelector('[data-testid="approval-history-empty"]') ||
            document.querySelector('[data-testid="approval-history-list"]')
          )
        )) as boolean,
      { timeout: 15_000 }
    );
  });
});
