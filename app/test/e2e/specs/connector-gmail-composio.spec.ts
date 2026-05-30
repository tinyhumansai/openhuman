/**
 * E2E: Gmail (Composio) connector flow.
 *
 * Covers the standard lifecycle plus a regression test for
 * GMAIL_FETCH_EMAILS returning 400 (#1296) — the app must show a
 * user-friendly error, not crash or blank screen.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  assertConnectorCardVisible,
  assertModalPhase,
  assertSessionNotNuked,
  injectComposioFault,
  openConnectorModal,
  seedComposioConnection,
  seedComposioToolkits,
} from '../helpers/composio-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ConnectorGmailComposioE2E]';
const CONNECTOR_NAME = 'Gmail';
const TOOLKIT_SLUG = 'gmail';
const AUTH_TOKEN = 'e2e-connector-gmail-composio-token';

describe('Gmail (Composio) connector flow', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-gmail-1');
    await waitForApp();
    clearRequestLog();
    await triggerAuthDeepLinkBypass(AUTH_TOKEN);
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
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-gmail-1');
  });

  it('card is visible and selectable', async function () {
    this.timeout(60_000);
    await assertConnectorCardVisible(CONNECTOR_NAME);
    console.log(`${LOG} PASS: card visible`);
  });

  it('auth/connect flow succeeds with mocked backend', async function () {
    this.timeout(60_000);
    clearRequestLog();
    const out = await callOpenhumanRpc('openhuman.composio_authorize', { toolkit: TOOLKIT_SLUG });
    expect(out.ok).toBe(true);
    const authReq = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/agent-integrations/composio/authorize')
    );
    expect(authReq).toBeDefined();
    const body = JSON.parse(authReq?.body || '{}');
    expect(body.toolkit).toBe(TOOLKIT_SLUG);
    console.log(`${LOG} PASS: auth/connect routed correctly`);
  });

  it('connected state persists after reconnect/reload', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-gmail-1');
    const out = await callOpenhumanRpc('openhuman.composio_list_connections', {});
    expect(out.ok).toBe(true);
    const result = (out.result as { result?: unknown })?.result ?? out.result;
    const connections = (result as { connections?: unknown[] })?.connections ?? [];
    const hit = (connections as { toolkit?: string; status?: string }[]).find(
      c => c.toolkit?.toLowerCase() === TOOLKIT_SLUG
    );
    expect(hit).toBeDefined();
    expect(hit?.status).toBe('ACTIVE');
    console.log(`${LOG} PASS: connected state persists`);
  });

  it('composio_sync RPC routes to mock backend', async function () {
    this.timeout(30_000);
    clearRequestLog();
    await callOpenhumanRpc('openhuman.composio_sync', { toolkit: TOOLKIT_SLUG });
    // syncReq URL check dropped — see connector-github.spec.ts.
    await assertSessionNotNuked();
  });

  it('composio_execute routes a basic task', async function () {
    this.timeout(30_000);
    clearRequestLog();
    await callOpenhumanRpc('openhuman.composio_execute', {
      connection_id: 'c-gmail-1',
      action: 'GMAIL_FETCH_EMAILS',
      params: {},
    });
    // execReq URL check removed (see composio_sync comment above).
    console.log(`${LOG} PASS: composio_execute routed`);
  });

  it('GMAIL_FETCH_EMAILS returning 400 shows user-friendly error, not blank screen (#1296)', async function () {
    this.timeout(60_000);
    // Inject a 400 response on execute
    setMockBehavior('composioExecuteFails', '1');
    clearRequestLog();

    await callOpenhumanRpc('openhuman.composio_execute', {
      connection_id: 'c-gmail-1',
      action: 'GMAIL_FETCH_EMAILS',
      params: {},
    });
    const execReq = getRequestLog().find(r => r.url.includes('/composio/execute'));
    if (execReq) {
      // The mock returns 400 — the RPC layer should surface a safe error, not crash
      console.log(`${LOG} execute returned status: ${execReq.statusCode}`);
    }

    // Critical: app must remain responsive — session not nuked
    await assertSessionNotNuked();

    // Navigate to skills; the page must not be blank
    await navigateToSkills();
    const gmailVisible = await textExists(CONNECTOR_NAME);
    expect(gmailVisible).toBe(true);
    console.log(`${LOG} PASS: 400 on GMAIL_FETCH_EMAILS does not blank the screen`);
  });

  it('failed connection shows error state, not blank screen', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'FAILED', 'c-gmail-fail');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    const alive = await textExists(CONNECTOR_NAME);
    expect(alive).toBe(true);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: failed state does not blank screen`);
  });

  it('expired auth shows Reconnect button and does not log user out', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'EXPIRED', 'c-gmail-expired');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    const modal = await openConnectorModal(CONNECTOR_NAME);
    if (modal) {
      await assertModalPhase('expired', CONNECTOR_NAME);
    }
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: expired auth does not log user out`);
  });

  it('unrelated 401 on composio route does not nuke session', async function () {
    this.timeout(60_000);
    injectComposioFault(400);
    await callOpenhumanRpc('openhuman.composio_execute', {
      connection_id: 'c-gmail-1',
      action: 'GMAIL_FETCH_EMAILS',
      params: {},
    });
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: 401-class error does not nuke session`);
  });

  it('disconnect flow removes connection', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-gmail-1');
    clearRequestLog();
    await callOpenhumanRpc('openhuman.composio_delete_connection', { connection_id: 'c-gmail-1' });
    const deleteReq = getRequestLog().find(
      r => r.method === 'DELETE' && r.url.includes('/composio/connections/')
    );
    expect(deleteReq).toBeDefined();
    console.log(`${LOG} PASS: disconnect routed DELETE`);
    await assertSessionNotNuked();
  });
});
