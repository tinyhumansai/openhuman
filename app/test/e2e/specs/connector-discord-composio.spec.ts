/**
 * E2E: Discord (Composio) connector flow.
 *
 * Covers Discord connector-card and connection-state UI.
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
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ConnectorDiscordComposioE2E]';
const CONNECTOR_NAME = 'Discord';
const TOOLKIT_SLUG = 'discord';
const AUTH_TOKEN = 'e2e-connector-discord-composio-token';

describe('Discord (Composio) connector flow', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-discord-1');
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
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-discord-1');
  });

  it('card is visible and selectable', async function () {
    this.timeout(60_000);
    await assertConnectorCardVisible(CONNECTOR_NAME);
    console.log(`${LOG} PASS: card visible`);
  });

  it('failed connection shows error state, not blank screen', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'FAILED', 'c-discord-fail');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    expect(await textExists(CONNECTOR_NAME)).toBe(true);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: failed state does not blank screen`);
  });

  // TODO(#6396): the Connections UI migration made modal discovery flaky on
  // Linux Wry; retain the other Discord session-safety coverage meanwhile.
  it.skip(// https://github.com/tinyhumansai/openhuman/issues/6396
  'expired auth shows Reconnect button and does not log user out', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'EXPIRED', 'c-discord-expired');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    const modal = await openConnectorModal(CONNECTOR_NAME, 15_000, 'Auth expired');
    expect(modal).toBeTruthy();
    await assertModalPhase('expired', CONNECTOR_NAME);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: expired auth does not log user out`);
  });
});
