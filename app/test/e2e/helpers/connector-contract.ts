/**
 * Table-driven Composio connector contract (plan.md §2.2).
 *
 * The per-toolkit connector specs shared one card/state flow, which remains
 * represented by Slack. Jira keeps its distinct subdomain-field UI.
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
  openConnectorModal,
  seedComposioConnection,
  seedComposioToolkits,
} from './composio-helpers';
import {
  clickButton,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from './element-helpers';
import { triggerAuthLoopbackBypass } from './loopback-auth-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from './shared-flows';

/**
 * One entry from the mock server's request log.
 *
 * `mock-server.ts` carries `@ts-nocheck` and re-exports `getRequestLog` from a
 * plain `.mjs` module, so the log arrives untyped and every predicate
 * parameter would otherwise be an implicit `any` (TS7006). Narrowing it here
 * keeps the three `.find(...)` / `.some(...)` call sites below typed without
 * touching the shared mock wrapper.
 */
interface MockRequestLogEntry {
  method: string;
  url: string;
}

export interface ConnectorContractConfig {
  /** Human-facing connector name as rendered in the UI (e.g. "Google Calendar"). */
  name: string;
  /** Composio toolkit slug (e.g. "googlecalendar"). */
  slug: string;
  /** Connection-id stem; the contract derives `${idBase}-1|-fail|-expired`. */
  idBase: string;
  /** Optional auth-deep-link token override (defaults to `e2e-connector-<slug>-token`). */
  authToken?: string;
}

/**
 * Register the shared Composio connector contract for one toolkit. Call once
 * per toolkit from the contract spec; each invocation is a self-contained
 * `describe` with its own mock lifecycle (mirrors the former per-file specs).
 */
export function runConnectorContract(config: ConnectorContractConfig): void {
  const { name, slug, idBase } = config;
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
      await triggerAuthLoopbackBypass(authToken);
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
      const modal = await openConnectorModal(name);
      expect(modal).toBeTruthy();
      console.log(`${LOG} PASS: card opens its connection UI`);
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

    // The case above stops at the affordance: it proves the Reconnect button
    // RENDERS. Nothing pressed it, so a regression that renders the button and
    // wires it to nothing — or to a route that 400s — left every connector
    // permanently un-reconnectable with the lane green. Matrix row 10.7.3
    // records that hole as "re-auth post-revoke not asserted".
    //
    // The button carries no `data-testid`, so this clicks its user-visible
    // label (`composio.reconnect` + the toolkit name, ComposioConnectModal.tsx
    // :245-253). A testid on that button would be the better hook; see the W5
    // phase-2 report.
    it('pressing Reconnect after expiry re-authorizes and restores the connection', async function () {
      this.timeout(60_000);
      seedComposioConnection(slug, 'EXPIRED', expiredId);
      await navigateToSkills();
      await waitForText(name, 10_000);
      const modal = await openConnectorModal(name, 15_000, 'Auth expired');
      expect(modal).toBeTruthy();
      await assertModalPhase('expired', name);

      // Cleared AFTER the modal is open so the assertion below can only be
      // satisfied by a request the button itself caused.
      clearRequestLog();
      await clickButton(`Reconnect ${name}`, 15_000);

      // The same evidence the happy-path connect case uses, so the two agree
      // on what "authorization was requested" means.
      await browser.waitUntil(
        async () =>
          (getRequestLog() as MockRequestLogEntry[]).some(
            r => r.method === 'POST' && r.url.includes('/composio/authorize')
          ),
        {
          timeout: 15_000,
          timeoutMsg:
            `Reconnect was pressed for ${name} but no POST to /composio/authorize followed — ` +
            'the expired-state button is not wired to the authorize route',
        }
      );

      await assertSessionNotNuked();
      console.log(`${LOG} PASS: reconnect after expiry re-authorizes`);
    });
  });
}
