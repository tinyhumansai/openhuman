// @ts-nocheck
/**
 * E2E test: invisible desktop runtime → provider login → onboarding/home → logout.
 *
 * Exercises the *first-launch login funnel* end-to-end against the shared
 * mock backend, running on the unified Appium chromium-driver session (CEF
 * over CDP) — the same harness CI uses for Linux in `e2e/docker-compose.yml`.
 *
 * The desktop app has exactly one runtime — the embedded local core —  and
 * BootCheckGate now commits to it silently instead of ever showing a picker
 * (see BootCheckGate.tsx). This spec used to drive that picker directly
 * (cloud URL/token validation, mode switching); that UI no longer exists on
 * desktop, so Phase 1 here now only asserts its absence. The equivalent
 * cloud-URL/token validation coverage lives in the web build's own boot-check
 * picker tests (BootCheckGate.test.tsx's "web (isTauri=false)" describe),
 * since a real remote-URL/token choice still exists there.
 *
 *   Phase 1 — No runtime picker:
 *     1. Reset to Welcome (no auth) and confirm no "Select a Runtime" button
 *        or picker heading is ever shown — the app already committed to the
 *        local core before Welcome rendered.
 *
 *   Phase 2 — Provider login (deep-link bypass simulates the OAuth round-trip):
 *     2. Welcome shows OAuth provider buttons. We don't click them (that opens
 *        the system browser), instead we simulate the post-OAuth deep link
 *        callback — exactly the same code path the real providers exercise
 *        when the backend redirects back to `openhuman://auth?token=...&key=auth`.
 *     3. Walk onboarding (if shown) until we reach Home.
 *     4. Verify mock backend recorded the auth/me profile fetch.
 *
 *   Phase 3 — Logout:
 *     5. Logout from Settings.
 *     6. Confirm we're back on Welcome (logged-out state visible), still with
 *        no runtime picker in sight.
 *
 * The mock server (scripts/mock-api-*) handles auth + profile + onboarding.
 * Deep links go through `window.__simulateDeepLink` so the spec is safe on
 * the headless Linux container — no system browser, no real OAuth round-trip,
 * and no PID-bound URL handler is touched.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  logoutViaSettings,
  waitForHomePage,
  waitForRequest,
  walkOnboarding,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[RuntimePicker]';

/** Neither the Welcome escape hatch nor the BootCheckGate picker heading exist on desktop. */
async function assertNoRuntimePicker(): Promise<void> {
  expect(await textExists('Select a Runtime')).toBe(false);
  expect(await textExists('Connect to Your Runtime')).toBe(false);
  expect(await textExists('Run Locally (Recommended)')).toBe(false);
  expect(await textExists('Run on the Cloud (Complex)')).toBe(false);
}

describe('Invisible desktop runtime → login → onboarding → home → logout', () => {
  before(async function beforeSuite() {
    // resetApp + app-ready can take longer than the default 30s per-hook budget.
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    // skipAuth so we land on Welcome (logged out) — the spec drives login
    // itself. clearAuthSession wipes the on-disk session token too, so a prior
    // login spec in this shard can't leave us authenticated (which would make
    // PublicRoute redirect past Welcome to /home).
    await resetApp('e2e-runtime-picker-login', { skipAuth: true, clearAuthSession: true });
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // -------------------------------------------------------------------------
  // Phase 1: no runtime picker anywhere in the normal flow
  // -------------------------------------------------------------------------

  it('app is running and shows Welcome with OAuth providers — no runtime picker', async function () {
    this.timeout(90_000);
    expect(await hasAppChrome()).toBe(true);
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    // Welcome.tsx: "Welcome to OpenHuman" title + at least one provider button.
    const welcomeShown = await waitForText('Welcome to OpenHuman', 15_000);
    if (!welcomeShown) {
      const tree = await dumpAccessibilityTree();
      console.log(`${LOG} Welcome not shown. Tree:\n`, tree.slice(0, 4000));
    }
    expect(welcomeShown).toBeTruthy();

    await assertNoRuntimePicker();
  });

  // -------------------------------------------------------------------------
  // Phase 2: Provider login (bypass deep link simulates the OAuth callback)
  // -------------------------------------------------------------------------

  it('OAuth provider buttons render on Welcome', async function () {
    this.timeout(90_000);
    // Real OAuth opens a system browser — out of scope for headless CI. We
    // just assert the buttons mount; the deep-link callback below covers the
    // post-OAuth path.
    const providerButtonPresent = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll('button'));
      return buttons.some(b => {
        const label = b.getAttribute('aria-label') || b.textContent || '';
        return /Google|GitHub|Twitter|Discord/i.test(label);
      });
    });
    expect(providerButtonPresent).toBe(true);
  });

  it('deep-link auth callback signs the user in and reaches Home', async function () {
    // Auth + onboarding + home confirmation needs more than 30s.
    this.timeout(90_000);
    clearRequestLog();
    await triggerAuthDeepLinkBypass('e2e-runtime-picker-user');
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(20_000);

    // The bypass path does not call the token-consume endpoint (it sets the
    // JWT directly) — that's by design. The /auth/me lookup MUST still fire.
    const meCall = await waitForRequest(getRequestLog, 'GET', '/auth/me', 20_000);
    if (!meCall) {
      console.log(`${LOG} /auth/me not seen. Log:`, JSON.stringify(getRequestLog(), null, 2));
    }
    expect(meCall).toBeTruthy();

    // Walk through onboarding if it's shown (new user path); a returning user
    // would skip directly to Home. walkOnboarding is a no-op when there's no
    // onboarding-next-button mounted.
    await walkOnboarding(LOG);

    // Confirm we're authenticated + post-onboarding. waitForHomePage's
    // hardcoded greeting strings (Good morning / Test / etc.) can miss
    // valid Home renders, so fall back to a route + welcome-gone check.
    const home = await waitForHomePage(15_000);
    if (home) {
      console.log(`${LOG} Home reached: "${home}"`);
    } else {
      const deadline = Date.now() + 15_000;
      let onHome = false;
      while (Date.now() < deadline) {
        const hash = (await browser.execute(() => window.location.hash)) as string;
        const stillOnWelcome = await textExists('Welcome to OpenHuman');
        if (!stillOnWelcome && (hash.startsWith('#/home') || hash.startsWith('#/chat'))) {
          onHome = true;
          break;
        }
        await browser.pause(500);
      }
      if (!onHome) {
        const tree = await dumpAccessibilityTree();
        console.log(`${LOG} Home not reached. Tree:\n`, tree.slice(0, 4000));
      }
      expect(onHome).toBe(true);
    }
  });

  // -------------------------------------------------------------------------
  // Phase 3: Logout returns to Welcome, still with no runtime picker
  // -------------------------------------------------------------------------

  it('logout from Settings returns the user to Welcome with no runtime picker', async function () {
    // Logout navigation + confirmation + wait for Welcome can take > 30s.
    this.timeout(60_000);
    await logoutViaSettings(LOG);

    // logoutViaSettings already asserts the logged-out marker; double-check
    // the Welcome OAuth row reappeared so we know the route reset cleanly,
    // and that returning to Welcome still never surfaces a runtime picker.
    expect(await waitForText('Welcome to OpenHuman', 15_000)).toBeTruthy();
    await assertNoRuntimePicker();
  });
});
