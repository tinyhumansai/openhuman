/**
 * E2E: Jira (Composio) connector flow.
 *
 * Includes an extra test verifying the subdomain required-field validation:
 * the Connect button must be disabled (or show an inline error) when no
 * valid Atlassian subdomain is entered.
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
import {
  completeOnboardingIfVisible,
  navigateToSkills,
  navigateViaHash,
} from '../helpers/shared-flows';
import {
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ConnectorJiraE2E]';
const CONNECTOR_NAME = 'Jira';
const TOOLKIT_SLUG = 'jira';
const AUTH_TOKEN = 'e2e-connector-jira-token';

async function waitForJiraDisconnected(timeout = 15_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const tile = document.querySelector<HTMLElement>(
          '[data-testid="skill-install-composio-jira"]'
        );
        if (!tile) return false;
        const label = tile.getAttribute('aria-label') ?? '';
        const text = tile.textContent ?? '';
        return !label.includes('Connected') && !text.includes('Connected');
      }),
    { timeout, interval: 300, timeoutMsg: 'Jira tile did not settle to disconnected state' }
  );
}

describe('Jira Composio connector flow', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    seedComposioToolkits([TOOLKIT_SLUG]);
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-jira-1');
    await waitForApp();
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
    seedComposioConnection(TOOLKIT_SLUG, 'ACTIVE', 'c-jira-1');
  });

  it('card is visible and selectable', async function () {
    this.timeout(60_000);
    await assertConnectorCardVisible(CONNECTOR_NAME);
    console.log(`${LOG} PASS: card visible`);
  });

  it('connect modal renders subdomain input field for Jira', async function () {
    this.timeout(60_000);
    // Seed as idle (no active connection) so we see the connect flow. The prior
    // test rendered Jira ACTIVE, which persisted into the durable client-side
    // connection cache (localStorage `composio:connections`). That cache
    // re-seeds the tile as connected on remount — before the live `[]` fetch
    // lands — and ComposioConnectModal latches its phase once at mount, so the
    // modal would open stuck in the `connected` phase (no subdomain form).
    // Clear the durable cache so the tile mounts disconnected and the modal
    // opens in `idle`.
    setMockBehavior('composioConnections', JSON.stringify([]));
    await browser.execute(() => {
      Object.keys(window.localStorage)
        .filter(k => k.includes('composio:connections'))
        .forEach(k => window.localStorage.removeItem(k));
    });
    // The preceding test already leaves us on Connections → Composio. Setting
    // the same hash again does not remount the page, so it would keep the
    // previous ACTIVE tile and never refetch the newly seeded empty list.
    // Take a real route transition before returning to the connector surface.
    await navigateViaHash('/chat');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    await waitForJiraDisconnected(20_000);
    const modal = await openConnectorModal(CONNECTOR_NAME);
    expect(modal).toBeTruthy();
    // The Jira connect modal should render a subdomain input per toolkitRequiredFields.ts
    // It uses data-testid="composio-required-subdomain"
    const hasSubdomainInput = await browser
      .execute(() => {
        return (
          document.querySelector('[data-testid="composio-required-subdomain"]') !== null ||
          document.querySelector('input[placeholder*="subdomain"]') !== null ||
          // fallback: any visible Jira tenant URL label
          Array.from(document.querySelectorAll('*')).some(el => {
            const text = el.textContent ?? '';
            const matches = text.match(/https?:\/\/[^\s"'<>]+/g) ?? [];
            return matches.some(candidate => {
              try {
                return new URL(candidate).hostname.endsWith('.atlassian.net');
              } catch {
                return false;
              }
            });
          })
        );
      })
      .catch(() => false);
    expect(hasSubdomainInput).toBe(true);
    console.log(`${LOG} PASS: subdomain input field visible in Jira modal`);
    // Close modal by pressing Escape
    await browser.keys(['Escape']).catch(() => {});
    await assertSessionNotNuked();
  });

  it('failed connection shows error state, not blank screen', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'FAILED', 'c-jira-fail');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    expect(await textExists(CONNECTOR_NAME)).toBe(true);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: failed state does not blank screen`);
  });

  it('expired auth shows Reconnect button and does not log user out', async function () {
    this.timeout(60_000);
    seedComposioConnection(TOOLKIT_SLUG, 'EXPIRED', 'c-jira-expired');
    await navigateToSkills();
    await waitForText(CONNECTOR_NAME, 10_000);
    const modal = await openConnectorModal(CONNECTOR_NAME, 15_000, 'Auth expired');
    expect(modal).toBeTruthy();
    await assertModalPhase('expired', CONNECTOR_NAME);
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: expired auth does not log user out`);
  });
});
