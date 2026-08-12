/**
 * End-to-end: webhook controller surface and compatibility-route coverage.
 *
 * The backend tunnel CRUD surface is available through OpenHuman. Echo-registration
 * behavior is exercised separately in webhooks-ingress-flow.spec.ts.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { resetMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-webhooks-tunnel';

describe('Webhook controller surface and retired-route coverage', () => {
  before(async function () {
    // resetApp bring-up can run ~25-30s and race the default 30s Mocha hook
    // budget; raise it.
    this.timeout(90_000);
    await startMockServer();
    await resetMockBehavior();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('reached the logged-in shell after onboarding', async () => {
    // Home.tsx: t('home.askAssistant') is the stable home page CTA button text.
    // After the /home → /chat redirect (AppRoutes.tsx), the chat new-window hero
    // renders t('home.statusOk') instead of the old CTA button.
    const atHome =
      (await textExists('Ask your assistant anything')) ||
      (await textExists('Your device is connected')) ||
      (await textExists('Your assistant is ready when you are')) ||
      (await textExists('Type something below to get started'));
    expect(atHome).toBe(true);
  });

  it('exposes backend tunnel CRUD', async () => {
    const listed = await callOpenhumanRpc('openhuman.webhooks_list_tunnels', {});
    expect(listed.ok).toBe(true);
  });

  it('legacy Webhooks route lands on Connections', async () => {
    // The dedicated Webhooks UI was retired. Keep the compatibility route
    // covered so old links land on the canonical Connections surface.
    await navigateViaHash('/webhooks');

    await browser.waitUntil(
      async () =>
        String(await browser.execute(() => window.location.hash)).includes('/connections'),
      { timeout: 10_000, interval: 500, timeoutMsg: 'Webhooks route did not reach Connections' }
    );

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/connections');
  });
});
