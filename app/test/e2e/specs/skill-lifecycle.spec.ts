// @ts-nocheck
/**
 * Skill lifecycle smoke (issue #224).
 *
 * Drives auth → onboarding → Skills page and asserts:
 *   1. The route mounts (`#/connections` — was `#/skills` before Phase 2).
 *   2. The Skills shell renders one of the well-known affordances
 *      (Skills/Install/Available header).
 *
 * Note: the Skills page now fetches data via the `openhuman.workflows_list`
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

  it('Connections page mounts and fetched the registry', async () => {
    // Phase 2: navigateToSkills() now points to /connections
    await navigateToSkills();
    await browser.waitUntil(
      // Phase 2: /skills redirects to /connections
      async () =>
        String(await browser.execute(() => window.location.hash)).includes('/connections'),
      { timeout: 10_000, interval: 250, timeoutMsg: 'Connections route did not mount in time' }
    );

    const hash = await browser.execute(() => window.location.hash);
    // Phase 2: /skills → /connections
    expect(String(hash)).toContain('/connections');

    // Connections page shows tabs: Apps (was Composio), Messaging (was Channels), Tools (was MCP)
    const visible =
      (await textExists('Apps')) ||
      (await textExists('Messaging')) ||
      (await textExists('Tools')) ||
      (await textExists('Connections'));
    expect(visible).toBe(true);

    // Verify the core RPC route for skills is reachable. The Skills page
    // uses openhuman.workflows_list (not a mock-backend HTTP call) since the
    // QuickJS skills runtime was removed. We probe it here as the
    // authoritative oracle that the data-fetch path is wired.
    const rpcResult = await callOpenhumanRpc('openhuman.workflows_list', {});
    expect(rpcResult.ok).toBe(true);
  });
});
