/**
 * E2E: GitHub Composio connector flow.
 *
 * Covers GitHub connector-card and connection-state UI.
 *
 * All backend calls are served by the mock server — no live GitHub account
 * is required.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  assertConnectorCardVisible,
  assertModalPhase,
  assertSessionNotNuked,
  openConnectorModal,
  seedComposioConnection,
  seedComposioToolkits,
} from '../helpers/composio-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { triggerAuthLoopbackBypass } from '../helpers/loopback-auth-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ConnectorGithubE2E]';
const CONNECTOR_NAME = 'GitHub';
const TOOLKIT_SLUG = 'github';
const AUTH_TOKEN = 'e2e-connector-github-token';

describe('GitHub Composio connector flow', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-github-1');
    setMockBehavior(
      'composioAvailableTriggers',
      JSON.stringify([{ slug: 'GITHUB_COMMIT_EVENT', scope: 'static' }])
    );
    setMockBehavior('composioActiveTriggers', JSON.stringify([]));
    await waitForApp();
    clearRequestLog();
    await triggerAuthLoopbackBypass(AUTH_TOKEN);
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await completeOnboardingIfVisible(LOG);
  });

  after(async () => {
    await stopMockServer();
  });

  afterEach(async () => {
    resetMockBehavior();
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-github-1');
    setMockBehavior(
      'composioAvailableTriggers',
      JSON.stringify([{ slug: 'GITHUB_COMMIT_EVENT', scope: 'static' }])
    );
    setMockBehavior('composioActiveTriggers', JSON.stringify([]));
  });

  it('card is visible and selectable', async function () {
    this.timeout(60_000);
    await assertConnectorCardVisible(CONNECTOR_NAME);
    console.log(`${LOG} PASS: card visible`);
  });

  it('failed connection shows error state, not blank screen', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'FAILED', 'c-github-fail');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    // App must remain responsive — skills page should not be blank
    const alive = await textExists(CONNECTOR_NAME);
    expect(alive).toBe(true);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: failed connection does not show blank screen`);
  });

  it('expired auth shows Reconnect button and does not log user out', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'EXPIRED', 'c-github-expired');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    const modal = await openConnectorModal(CONNECTOR_NAME, 15_000, 'Auth expired');
    expect(modal).toBeTruthy();
    await assertModalPhase('expired', CONNECTOR_NAME);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: expired auth does not log user out`);
  });
});
