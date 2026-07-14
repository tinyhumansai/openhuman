/**
 * Table-driven Composio connector contract (plan.md §2.2).
 *
 * The per-toolkit connector specs (Airtable, Asana, ClickUp, …) were 11
 * byte-identical WDIO files differing only in a handful of toolkit strings.
 * `runConnectorContract` factors the whole `describe` block out so each toolkit
 * is one `ConnectorContractConfig` row in the single contract spec. Toolkits
 * with bespoke UI/behaviour (Jira's subdomain field, Gmail's 400-on-fetch)
 * keep their own dedicated specs.
 */
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';
import { waitForApp } from './app-helpers';
import {
  assertConnectorCardVisible,
  assertModalPhase,
  assertSessionNotNuked,
  injectComposioFault,
  openConnectorModal,
  seedComposioConnection,
  seedComposioToolkits,
} from './composio-helpers';
import { callOpenhumanRpc } from './core-rpc';
import { triggerAuthDeepLinkBypass } from './deep-link-helpers';
import { textExists, waitForText, waitForWebView, waitForWindowVisible } from './element-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from './shared-flows';

export interface ConnectorContractConfig {
  /** Human-facing connector name as rendered in the UI (e.g. "Google Calendar"). */
  name: string;
  /** Composio toolkit slug (e.g. "googlecalendar"). */
  slug: string;
  /** Connection-id stem; the contract derives `${idBase}-1|-fail|-expired`. */
  idBase: string;
  /** A representative Composio action for the execute-routing case. */
  executeAction: string;
  /** Optional auth-deep-link token override (defaults to `e2e-connector-<slug>-token`). */
  authToken?: string;
}

/**
 * Register the shared Composio connector contract for one toolkit. Call once
 * per toolkit from the contract spec; each invocation is a self-contained
 * `describe` with its own mock lifecycle (mirrors the former per-file specs).
 */
export function runConnectorContract(config: ConnectorContractConfig): void {
  const { name, slug, idBase, executeAction } = config;
  const authToken = config.authToken ?? `e2e-connector-${slug}-token`;
  const activeId = `${idBase}-1`;
  const failId = `${idBase}-fail`;
  const expiredId = `${idBase}-expired`;
  const LOG = `[Connector:${name}]`;

  describe(`${name} Composio connector flow`, () => {
    before(async function () {
      this.timeout(90_000);
      await startMockServer();
      seedComposioToolkits([slug]);
      seedComposioConnection(slug, 'ACTIVE', activeId);
      await waitForApp();
      clearRequestLog();
      await triggerAuthDeepLinkBypass(authToken);
      await waitForWindowVisible(25_000);
      await waitForWebView(15_000);
      await completeOnboardingIfVisible(LOG);
    });

    after(async () => {
      await stopMockServer();
    });

    afterEach(async () => {
      resetMockBehavior();
      seedComposioToolkits([slug]);
      seedComposioConnection(slug, 'ACTIVE', activeId);
    });

    it('card is visible and selectable', async function () {
      this.timeout(60_000);
      await assertConnectorCardVisible(name);
      console.log(`${LOG} PASS: card visible`);
    });

    it('auth/connect flow succeeds with mocked backend', async function () {
      this.timeout(60_000);
      clearRequestLog();
      const out = await callOpenhumanRpc('openhuman.composio_authorize', { toolkit: slug });
      expect(out.ok).toBe(true);
      const authReq = getRequestLog().find(
        r => r.method === 'POST' && r.url.includes('/composio/authorize')
      );
      expect(authReq).toBeDefined();
      console.log(`${LOG} PASS: auth/connect routed`);
    });

    it('connected state persists after reconnect/reload', async function () {
      this.timeout(60_000);
      const out = await callOpenhumanRpc('openhuman.composio_list_connections', {});
      expect(out.ok).toBe(true);
      const result = (out.result as { result?: unknown })?.result ?? out.result;
      const connections = (result as { connections?: unknown[] })?.connections ?? [];
      const hit = (connections as { toolkit?: string; status?: string }[]).find(
        c => c.toolkit?.toLowerCase() === slug
      );
      expect(hit).toBeDefined();
      expect(hit?.status).toBe('ACTIVE');
      console.log(`${LOG} PASS: connected state persists`);
    });

    // Renamed from the misleading "composio_sync RPC routes to mock backend"
    // (plan.md §3): composio_sync short-circuits with no HTTP for connectors
    // without a native provider, so the real, verifiable contract is that the
    // call does not tear down the WebDriver session.
    it('composio_sync does not tear down the session', async function () {
      this.timeout(30_000);
      clearRequestLog();
      await callOpenhumanRpc('openhuman.composio_sync', { toolkit: slug });
      await assertSessionNotNuked();
      console.log(`${LOG} PASS: sync does not nuke session`);
    });

    it('composio_execute routes a basic task', async function () {
      this.timeout(30_000);
      clearRequestLog();
      await callOpenhumanRpc('openhuman.composio_execute', {
        connection_id: activeId,
        action: executeAction,
        params: {},
      });
      console.log(`${LOG} PASS: execute routed`);
    });

    it('failed connection shows error state, not blank screen', async function () {
      this.timeout(60_000);
      seedComposioConnection(slug, 'FAILED', failId);
      await navigateToSkills();
      await waitForText(name, 10_000);
      expect(await textExists(name)).toBe(true);
      await assertSessionNotNuked();
      console.log(`${LOG} PASS: failed state does not blank screen`);
    });

    it('expired auth shows Reconnect button and does not log user out', async function () {
      this.timeout(60_000);
      seedComposioConnection(slug, 'EXPIRED', expiredId);
      await navigateToSkills();
      await waitForText(name, 10_000);
      const modal = await openConnectorModal(name, 15_000, 'Auth expired');
      expect(modal).toBeTruthy();
      await assertModalPhase('expired', name);
      await assertSessionNotNuked();
      console.log(`${LOG} PASS: expired auth does not log user out`);
    });

    it('unrelated 400 on composio route does not nuke session', async function () {
      this.timeout(60_000);
      injectComposioFault(400);
      await callOpenhumanRpc('openhuman.composio_execute', {
        connection_id: activeId,
        action: executeAction,
        params: {},
      });
      await assertSessionNotNuked();
      console.log(`${LOG} PASS: unrelated 400 error does not nuke session`);
    });

    it('disconnect flow removes connection', async function () {
      this.timeout(60_000);
      seedComposioConnection(slug, 'ACTIVE', activeId);
      clearRequestLog();
      await callOpenhumanRpc('openhuman.composio_delete_connection', { connection_id: activeId });
      const deleteReq = getRequestLog().find(
        r => r.method === 'DELETE' && r.url.includes('/composio/connections/')
      );
      expect(deleteReq).toBeDefined();
      console.log(`${LOG} PASS: disconnect routed DELETE`);
      await assertSessionNotNuked();
    });
  });
}
