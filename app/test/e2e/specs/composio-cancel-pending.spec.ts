/**
 * E2E: cancelling a Composio OAuth handoff that is stuck "Connecting".
 *
 * The unit tests around `useComposioConnectFlow` mock `composioApi`, so they
 * prove the hook's state machine but not that the button reaches the backend.
 * This spec drives the real path: a toolkit seeded as `INITIATED` renders the
 * tile as "Connecting", the modal resumes its poll on open, and Cancel has to
 * issue the delete through the core RPC and the mock backend before the modal
 * may claim the handoff is gone.
 *
 * Regression guard for the stuck-pending state: before the cancel control the
 * only exit was closing the modal, which left the connection PENDING, so the
 * tile kept reading "Connecting" and reopening the modal resumed the wait.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  assertModalPhase,
  openConnectorModal,
  seedComposioConnection,
  seedComposioToolkits,
} from '../helpers/composio-helpers';
import {
  clickTestId,
  waitForTestId,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { triggerAuthLoopbackBypass } from '../helpers/loopback-auth-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ComposioCancelPendingE2E]';
const CONNECTOR_NAME = 'Gmail';
const TOOLKIT_SLUG = 'gmail';
const PENDING_CONNECTION_ID = 'c-gmail-pending';
const AUTH_TOKEN = 'e2e-composio-cancel-pending-token';

describe('Composio pending-connection cancel flow', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    seedComposioToolkits([TOOLKIT_SLUG]);
    // INITIATED is what Composio reports while the browser handoff is open.
    seedComposioConnection(TOOLKIT_SLUG, 'INITIATED', PENDING_CONNECTION_ID);
    await waitForApp();
    clearRequestLog();
    await triggerAuthLoopbackBypass(AUTH_TOKEN);
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await completeOnboardingIfVisible(LOG);
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  it('deletes the pending connection and returns the modal to idle', async function () {
    this.timeout(120_000);

    await navigateToSkills();
    const marker = await openConnectorModal(CONNECTOR_NAME, 20_000, 'Connecting');
    if (!marker) throw new Error(`${LOG} connector modal for ${CONNECTOR_NAME} did not open`);

    // The modal must resume the handoff rather than offering Connect again.
    await assertModalPhase('waiting', CONNECTOR_NAME, 20_000);

    clearRequestLog();
    await waitForTestId('composio-cancel-connect', 15_000);
    await clickTestId('composio-cancel-connect');

    // The delete has to reach the backend — a local-only reset would leave the
    // handoff open on the Composio side.
    const deadline = Date.now() + 30_000;
    let deleteSeen = false;
    while (Date.now() < deadline && !deleteSeen) {
      deleteSeen = getRequestLog().some(
        (entry: { method: string; url: string }) =>
          entry.method === 'DELETE' &&
          entry.url.includes('/agent-integrations/composio/connections') &&
          entry.url.includes(PENDING_CONNECTION_ID)
      );
      if (deleteSeen) break;
      await browser.pause(500);
    }
    if (!deleteSeen) {
      throw new Error(`${LOG} no DELETE for ${PENDING_CONNECTION_ID} reached the mock backend`);
    }
    console.log(`${LOG} delete for ${PENDING_CONNECTION_ID} observed at the backend`);

    // With the row gone the modal offers a fresh connect instead of waiting.
    await assertModalPhase('idle', CONNECTOR_NAME, 20_000);
  });
});
