import { waitForApp } from '../helpers/app-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

describe('Slack connection entry point', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) this.skip();
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-slack-flow');
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens the current messaging connections surface', async () => {
    await navigateViaHash('/connections?tab=messaging');
    await browser.waitUntil(async () => (await browser.getUrl()).includes('#/connections'), {
      timeout: 10_000,
      timeoutMsg: 'messaging connections surface did not open',
    });
  });
});
