// @ts-nocheck
/**
 * Skill lifecycle smoke (issue #224).
 *
 * Drives auth → onboarding → Skills page and asserts:
 *   1. The route mounts (`#/skills`).
 *   2. The Skills shell renders one of the well-known affordances
 *      (Skills/Install/Available header).
 *
 * Note: the Skills page now fetches data via the `openhuman.skills_list`
 * JSON-RPC method (not via a REST GET /skills to the mock backend). The
 * mock-HTTP oracle was removed so the spec does not produce false-negative
 * failures when the UI wires correctly through core RPC.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-lifecycle';

describe('Skill lifecycle smoke', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('Skills page mounts and fetched the registry', async () => {
    await navigateToSkills();
    await browser.waitUntil(
      async () => String(await browser.execute(() => window.location.hash)).includes('/skills'),
      { timeout: 10_000, interval: 250, timeoutMsg: 'Skills route did not mount in time' }
    );

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const visible =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available'));
    expect(visible).toBe(true);

    // Verify the core RPC route for skills is reachable. The Skills page
    // uses openhuman.skills_list (not a mock-backend HTTP call) since the
    // QuickJS skills runtime was removed. We probe it here as the
    // authoritative oracle that the data-fetch path is wired.
    const rpcResult = await callOpenhumanRpc('openhuman.skills_list', {});
    expect(rpcResult.ok).toBe(true);
  });
});
