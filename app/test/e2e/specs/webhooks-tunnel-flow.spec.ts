/**
 * End-to-end: webhook controller surface and compatibility-route coverage.
 *
 * The backend tunnel CRUD surface is available through OpenHuman. Echo-registration
 * behavior is exercised separately in webhooks-ingress-flow.spec.ts.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
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
    // The assistant-ui chat view intentionally has no fixed greeting copy.
    // The authenticated sidebar is the stable shell marker after onboarding.
    const atShell = await browser.execute(
      () => document.querySelector('[data-testid="root-shell-sidebar"]') !== null
    );
    expect(atShell).toBe(true);
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
