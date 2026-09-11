import { waitForApp } from '../helpers/app-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/** Covers the legacy Accounts route while provider connection moved to Connections. */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[WhatsAppFlowE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[WhatsAppFlowE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('WhatsApp account integration smoke', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — route assertions require script execution');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('resetting app');
    await resetApp('e2e-whatsapp-flow');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('redirects the retired accounts route to unified chat', async () => {
    await navigateViaHash('/accounts');
    await browser.waitUntil(async () => (await browser.getUrl()).includes('#/chat'), {
      timeout: 5_000,
      timeoutMsg: 'retired /accounts route did not redirect to unified chat',
    });
  });
});
