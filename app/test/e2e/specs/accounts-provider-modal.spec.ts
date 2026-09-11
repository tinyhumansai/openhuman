import { waitForApp } from '../helpers/app-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

describe('Legacy accounts route', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) this.skip();
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-legacy-accounts-route');
  });

  after(async () => {
    await stopMockServer();
  });

  it('redirects the retired provider picker route to unified chat', async () => {
    await navigateViaHash('/accounts');
    await browser.waitUntil(async () => (await browser.getUrl()).includes('#/chat'), {
      timeout: 5_000,
      timeoutMsg: 'retired /accounts route did not redirect to unified chat',
    });
  });
});
