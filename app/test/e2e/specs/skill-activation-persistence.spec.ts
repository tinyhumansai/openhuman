/**
 * E2E: skill activation persists across a (simulated) app restart (issue #4273, AC1).
 *
 * The Skills page fetches Composio connections fresh on every mount, so a cold
 * start used to flash an empty/disconnected grid until the first backend
 * round-trip landed. The durable connection cache (`connectionCache.ts`) fixes
 * that by seeding the last-known activation state instantly on mount.
 *
 * This spec proves the activated state survives a restart-equivalent re-mount
 * EVEN WHEN the backend is unreachable at that moment — which is exactly the
 * cold-start window the cache exists to cover. We:
 *   1. Seed an ACTIVE Composio connection and load the Skills page once so the
 *      durable cache is populated.
 *   2. Inject a Composio backend fault so any *fresh* fetch would fail.
 *   3. Re-mount the page (navigate away + back — the restart-equivalent the
 *      rewards-persistence spec also uses, since tauri-driver has no cheap real
 *      restart) and assert the activated skill still renders, served from the
 *      cache rather than a successful new fetch.
 *
 * If the card only ever came from a live fetch, step 3 would show it missing
 * once the fault is injected — so a passing assertion is specifically
 * attributable to the persisted cache.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  assertConnectorCardVisible,
  assertSessionNotNuked,
  injectComposioFault,
  seedComposioConnection,
  seedComposioToolkits,
} from '../helpers/composio-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  navigateToConnections,
  navigateViaHash,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[skill-activation-persistence]';
const CONNECTOR_NAME = 'Gmail';
const TOOLKIT_SLUG = 'gmail';
const AUTH_TOKEN = 'e2e-skill-activation-persistence-token';

/**
 * Restart-equivalent: navigate away so the Skills page unmounts, then back so
 * it re-mounts and re-runs its on-mount fetch — the same approach
 * `rewards-progression-persistence.spec.ts` uses in lieu of a real process
 * restart (which tauri-driver does not support cheaply).
 */
async function simulateRestart(): Promise<void> {
  await navigateViaHash('/home');
  await browser.pause(1_000);
  await navigateToConnections();
  await browser.pause(1_000);
}

describe('Skill activation persistence across restart', () => {
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
    resetMockBehavior();
    await stopMockServer();
  });

  it('shows the activated skill on first load and writes it to the durable cache', async function () {
    this.timeout(60_000);
    await assertConnectorCardVisible(CONNECTOR_NAME);

    // Durable-write proof (PR #4288 review): the connection must be persisted to
    // localStorage under the user-scoped `${userId}:composio:connections:v1`
    // key — not merely held in the module's in-memory mirror. Asserting the
    // durable blob here makes the persistence path load-bearing for this spec,
    // so a broken write fails the test instead of being masked by the in-memory
    // hydrate on the re-mount below. (Cold-restart read-back — fresh module
    // memory, warm localStorage — is covered by the connectionCache unit test,
    // which tauri-driver cannot cheaply reproduce with a real relaunch.)
    const persisted = await browser.execute(() => {
      // Resolve the active user id and read that exact user-scoped key, rather
      // than suffix-matching any cache entry — otherwise a stale blob from a
      // different user could satisfy the assertion (PR #4288 review).
      const userId = window.localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
      return userId ? window.localStorage.getItem(`${userId}:composio:connections:v1`) : null;
    });
    expect(persisted).toBeTruthy();
    expect(String(persisted).toLowerCase()).toContain(TOOLKIT_SLUG);
    console.log(`${LOG} PASS: activated skill visible + persisted to durable cache`);
  });

  it('still shows the activated skill after a restart when the backend is unreachable', async function () {
    this.timeout(60_000);

    // From here on, any fresh Composio fetch fails — so a card that appears
    // after the re-mount came from the seeded/persisted state, not a new
    // backend fetch.
    injectComposioFault(500);

    await simulateRestart();

    await waitForText(CONNECTOR_NAME, 15_000);
    expect(await textExists(CONNECTOR_NAME)).toBe(true);
    // The unreachable backend must not blank the page or tear down the session.
    await assertSessionNotNuked();
    console.log(`${LOG} PASS: activation survived restart via the durable cache`);
  });
});
