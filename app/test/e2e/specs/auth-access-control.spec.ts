/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Authentication & Access Control + Billing & Subscriptions (Linux / tauri-driver).
 *
 * Covers:
 *   1.1    User registration via deep link
 *   1.1.1  Duplicate account handling (re-auth same user)
 *   1.2    Multi-device sessions (second JWT accepted)
 *   3.1.1  Billing dashboard handoff is available
 *   3.2.1  Billing dashboard entry point is stable
 *   3.3.1  Subscription management handoff is displayed
 *   3.3.3  Manage subscription uses the web dashboard handoff
 *   1.3    Logout via Settings menu
 *   1.3.1  Revoked session auto-logout
 *
 * Onboarding steps:
 *   Welcome → Skills → optional Context. The shared helper accepts older
 *   onboarding copy as fallback so this spec keeps covering auth/billing.
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc, expectRpcOk } from '../helpers/core-rpc';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  clickText,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  navigateToBilling,
  navigateToHome,
  navigateToSettings,
  navigateViaHash,
  waitForHomePage,
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

// waitForHomePage imported from shared-flows

async function waitForTextToDisappear(text, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await textExists(text))) return true;
    await browser.pause(500);
  }
  return false;
}

async function waitForRequest(method, urlFragment, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const log = getRequestLog();
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

async function expectBillingMarkers(markers) {
  const results = [];
  for (const marker of markers) {
    results.push([marker, await textExists(marker)]);
  }
  const missing = results.filter(([, found]) => !found).map(([marker]) => marker);
  if (missing.length > 0) {
    console.log('[AuthAccess] Billing request log:', JSON.stringify(getRequestLog(), null, 2));
    const tree = await dumpAccessibilityTree();
    console.log('[AuthAccess] Billing page tree:\n', tree.slice(0, 6000));
  }
  for (const [marker, found] of results) {
    expect(found).toBe(true);
    console.log(`[AuthAccess] Billing marker verified: ${marker}`);
  }
}

// walkOnboarding, waitForHomePage imported from shared-flows

/**
 * Perform full login via deep link. Walks onboarding. Leaves app on Home page.
 */
async function performFullLogin(token = 'e2e-test-token') {
  await triggerAuthDeepLink(token);

  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);
  await waitForAuthBootstrap(15_000);

  const consumeCall = await waitForRequest('POST', '/auth/login-token/consume', 20_000);
  if (!consumeCall) {
    console.log(
      '[AuthAccess] Missing consume call. Request log:',
      JSON.stringify(getRequestLog(), null, 2)
    );
    throw new Error('Auth consume call missing in performFullLogin');
  }
  // The app may call /auth/me or /settings for user profile
  const meCall =
    (await waitForRequest('GET', '/auth/me', 10_000)) ||
    (await waitForRequest('GET', '/settings', 10_000));
  if (!meCall) {
    console.log(
      '[AuthAccess] Missing user profile call. Request log:',
      JSON.stringify(getRequestLog(), null, 2)
    );
    console.log('[AuthAccess] Continuing without user profile call confirmation');
  }

  // Walk real onboarding steps
  await walkOnboarding('[AuthAccess]');

  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log('[AuthAccess] Home page not reached after login. Tree:\n', tree.slice(0, 4000));
    throw new Error('Full login did not reach Home page');
  }
  console.log(`[AuthAccess] Home page confirmed: found "${homeText}"`);
}

/**
 * `AuthStateResponse` — `crates/openhuman-core/src/security/credentials/responses.rs`.
 *
 * `credential` is `skip_serializing_if = "Option::is_none"` on the Rust side,
 * so it is absent (not null) when signed out.
 */
interface AuthStateResponse {
  isAuthenticated: boolean;
  userId?: string | null;
  credential?: 'session' | 'api-key' | 'local';
}

function authStateValue(state: { result?: unknown }): AuthStateResponse {
  const raw = (state.result ?? {}) as { value?: AuthStateResponse } & AuthStateResponse;
  return raw.value ?? raw;
}

// ===========================================================================
// Test suite
// ===========================================================================

describe('Auth & Access Control', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await waitForApp();
    // Wipe prior-spec state but stop before auth — this spec drives the
    // login flow itself via `performFullLogin`, so it has to start from
    // a logged-out Welcome screen.
    await resetApp('e2e-auth-access-reset', { skipAuth: true });
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // -------------------------------------------------------------------------
  // 1. Authentication
  // -------------------------------------------------------------------------

  it('new user registers via deep link and reaches home', async function () {
    this.timeout(120_000);
    await performFullLogin('e2e-auth-token');
  });

  it('re-authenticating with a new token for the same user returns to home', async () => {
    clearRequestLog();
    await triggerAuthDeepLink('e2e-auth-reauth-token');

    // Wait until the app has processed the deep-link and navigated away from
    // any loading state — poll for a home marker or the auth token consume
    // request, whichever comes first.
    await browser.waitUntil(
      async () => {
        const homeText = await waitForHomePage(500);
        if (homeText) return true;
        const consumed = getRequestLog().find(
          r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
        );
        return !!consumed;
      },
      {
        timeout: 10_000,
        interval: 500,
        timeoutMsg: 'Timed out waiting for re-auth deep-link to be processed',
      }
    );

    const homeText = await waitForHomePage(15_000);
    if (!homeText) {
      await navigateToHome();
    }
    const finalHome = homeText || (await waitForHomePage(10_000));
    expect(finalHome).not.toBeNull();
    console.log('[AuthAccess] Re-auth completed, on Home');
  });

  it('second device token is accepted and processed', async () => {
    clearRequestLog();
    await triggerAuthDeepLink('e2e-auth-device2-token');

    // Wait for the deep-link to be consumed before asserting home state.
    await browser.waitUntil(
      async () => {
        const consumed = getRequestLog().find(
          r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
        );
        return !!consumed;
      },
      {
        timeout: 10_000,
        interval: 500,
        timeoutMsg: 'Timed out waiting for device-2 token consume call',
      }
    );

    const homeText = await waitForHomePage(15_000);
    if (!homeText) {
      await navigateToHome();
    }
    const finalHome = homeText || (await waitForHomePage(10_000));
    expect(finalHome).not.toBeNull();

    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
    );
    expect(consumeCall).toBeDefined();
    console.log('[AuthAccess] Multi-device token accepted');
  });

  // -------------------------------------------------------------------------
  // 2. Default Plan
  // -------------------------------------------------------------------------

  it.skip('3.1.1 — billing dashboard handoff is available', async () => {
    await navigateToBilling();
    // The upstream billing summary now arrives asynchronously. Reuse the
    // polling marker assertion so this first navigation has the same contract
    // as the later billing checks in this spec.
    await expectBillingMarkers(['Open billing dashboard']);

    console.log('[AuthAccess] 3.1.1 — Billing web handoff verified');
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 3. Upgrade Flow
  // -------------------------------------------------------------------------

  it.skip('3.2.1 — billing dashboard entry point is stable', async () => {
    await navigateToBilling();
    clearRequestLog();

    await expectBillingMarkers(['Open billing dashboard', 'TinyHumans on the web']);

    console.log('[AuthAccess] 3.2.1 — Billing dashboard entry point verified');
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 4. Active Subscription Display
  // -------------------------------------------------------------------------

  it.skip('3.3.1 — subscription management handoff is displayed correctly', async () => {
    // Seed mock state explicitly so this test is self-contained
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());
    clearRequestLog();

    await navigateToBilling();

    await expectBillingMarkers([
      'Billing moved to the web',
      'Subscription changes',
      'Open billing dashboard',
    ]);

    console.log('[AuthAccess] 3.3.1 — Subscription management handoff verified');
  });

  it.skip('3.3.3 — manage subscription uses the web dashboard handoff', async () => {
    // Seed mock state explicitly so this test is self-contained
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());
    clearRequestLog();

    await navigateToBilling();
    await browser.pause(3_000);

    await expectBillingMarkers(['Open billing dashboard']);

    console.log('[AuthAccess] 3.3.3 — Dashboard handoff verified');
    resetMockBehavior();
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 5. Logout
  // -------------------------------------------------------------------------

  it.skip('user can log out via Settings and returns to Welcome', async () => {
    // resetApp established a clean authenticated session for this suite.
    // A second asynchronous deep-link login here races its post-login redirect
    // against the Settings navigation, while adding no logout coverage.
    await navigateToHome();

    // Log out + Clear App Data moved out of the main /settings page and
    // into the Account section in PR #2550 (LogoutAndClearActions footer
    // on /settings/account).
    await navigateViaHash('/settings/account');

    // Click "Log out" via JS — the settings menu item text is "Log out"
    // with description "Sign out of your account"
    const loggedOut = await browser.execute(() => {
      const allElements = document.querySelectorAll('*');
      for (const el of allElements) {
        const text = el.textContent?.trim() || '';
        if (text === 'Log out') {
          const clickable = el.closest(
            'button, [role="button"], a, [class*="MenuItem"]'
          ) as HTMLElement;
          if (clickable) {
            clickable.click();
            return 'clicked-parent';
          }
          (el as HTMLElement).click();
          return 'clicked-self';
        }
      }
      return null;
    });

    if (!loggedOut) {
      // Fallback: try XPath text search
      const logoutCandidates = ['Log out', 'Logout', 'Sign out'];
      let found = false;
      for (const text of logoutCandidates) {
        if (await textExists(text)) {
          await clickText(text, 10_000);
          console.log(`[AuthAccess] Clicked "${text}" via XPath`);
          found = true;
          break;
        }
      }
      if (!found) {
        const tree = await dumpAccessibilityTree();
        console.log('[AuthAccess] Logout button not found. Tree:\n', tree.slice(0, 4000));
        throw new Error('Could not find logout button in Settings');
      }
    } else {
      console.log(`[AuthAccess] Logout: ${loggedOut}`);
    }

    // If a confirmation dialog appears, confirm it
    await browser.pause(2_000);
    const hasConfirm =
      (await textExists('Confirm')) || (await textExists('Yes')) || (await textExists('Log Out'));
    if (hasConfirm) {
      const confirmed = await browser.execute(() => {
        const candidates = document.querySelectorAll('button, [role="button"], a');
        for (const el of candidates) {
          const text = el.textContent?.trim() || '';
          const label = el.getAttribute('aria-label') || '';
          if (['Confirm', 'Yes', 'Log Out'].some(t => text === t || label === t)) {
            (el as HTMLElement).click();
            return true;
          }
        }
        return false;
      });
      expect(confirmed).toBe(true);
      console.log('[AuthAccess] Confirmation dialog: clicked');
      await browser.pause(2_000);
    }

    // ── Assertion, rewritten ────────────────────────────────────────────
    //
    // This used to be `expect(onWelcome || !hasToken).toBe(true)`, where
    // `hasToken` read `localStorage['persist:auth']`. That key does not
    // exist and has not for some time: `app/src/store/index.ts` registers no
    // `auth` reducer and no `auth` persist config, and this suite's sibling
    // says so in a comment (`login-flow.spec.ts`, bypass case). So `hasToken`
    // was always `false`, `!hasToken` was always `true`, and the disjunction
    // was a tautology — the test passed whether or not logout did anything.
    // Matrix row 1.4.1 was marked green on that.
    //
    // Two independent mechanisms now, both required:
    //   1. the core no longer holds a credential (the half that matters for
    //      security: a UI that routes to Welcome while the core keeps
    //      authenticating is exactly the regression worth catching), and
    //   2. the shell actually renders the logged-out surface.
    await browser.pause(3_000);

    const state = await callOpenhumanRpc<AuthStateResponse>('openhuman.auth_get_state', {});
    expectRpcOk('auth_get_state', state);
    const authState = authStateValue(state);
    expect(authState.isAuthenticated).toBe(false);
    // `credential` is `skip_serializing_if = "Option::is_none"` on the Rust
    // side, so a signed-out state omits it entirely. A stale "session" or
    // "local" value here means the credential outlived the logout.
    expect(authState.credential).toBeUndefined();
    console.log('[AuthAccess] Logout: core reports no credential');

    // `'OpenHuman'` is deliberately NOT in this list. It appears 148 times in
    // `app/src/lib/i18n/en.ts` and `textExists` is an unanchored
    // `//*[contains(text(), …)]`, so including it would match on most screens
    // and re-create the same always-true assertion in a new disguise.
    const welcomeCandidates = ['Welcome', 'Sign in', 'Login', 'Get Started'];
    let onWelcome = false;
    for (const text of welcomeCandidates) {
      if (await textExists(text)) {
        console.log(`[AuthAccess] Logged-out state confirmed: found "${text}"`);
        onWelcome = true;
        break;
      }
    }
    expect(onWelcome).toBe(true);
  });

  it.skip('revoked session auto-logs out the user', async function () {
    this.timeout(120_000);
    // Login fresh
    clearRequestLog();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await performFullLogin('e2e-revoked-session-token');

    // Set mock to return 401 for user profile requests (revoked session)
    setMockBehavior('session', 'revoked');

    // Trigger a re-auth which will fail with 401
    await triggerAuthDeepLink('e2e-revoked-check-token');

    // Wait for the app to process the revoked token. The app should either
    // navigate away from Home (auto-logout) or the token consume call should
    // arrive. Poll with a generous timeout since 401 handling involves an
    // async auth state update.
    await browser.waitUntil(
      async () => {
        // Either the app has logged us out (no home markers) or the
        // consume request arrived so we can proceed to the assertion.
        const homeText = await waitForHomePage(500);
        if (!homeText) return true; // navigated away — auto-logout happened
        const consumed = getRequestLog().find(
          r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
        );
        return !!consumed;
      },
      {
        timeout: 12_000,
        interval: 500,
        timeoutMsg: 'Timed out waiting for revoked-session response',
      }
    );

    // ── Assertion, rewritten ────────────────────────────────────────────
    //
    // Was `expect(onWelcome || !stillOnHome).toBe(true)` with `'OpenHuman'`
    // in the `onWelcome` candidate list. `'OpenHuman'` occurs 148 times in
    // `app/src/lib/i18n/en.ts` and `textExists` matches any text node
    // containing it, so the first disjunct was satisfiable on essentially
    // any screen — including the Home the test was supposed to prove we had
    // left. The test could not distinguish "revocation propagated" from
    // "revocation did nothing".
    //
    // Three assertions now, and none of them is a disjunction:
    //   1. the mock actually served the 401 (proves the fault was injected —
    //      without this the whole test can pass because nothing was revoked),
    //   2. the core dropped the credential,
    //   3. the shell left the authenticated surface.

    // 1. The injection landed. A revoked-session test that never provoked a
    //    401 is the "couldn't-run wearing the clothes of proved" failure:
    //    everything downstream would look like a clean auto-logout.
    const meCalls = getRequestLog().filter(r => r.method === 'GET' && r.url.includes('/auth/me'));
    expect(meCalls.length).toBeGreaterThan(0);

    // 2. The core dropped the credential.
    await browser.waitUntil(
      async () => {
        const state = await callOpenhumanRpc<AuthStateResponse>('openhuman.auth_get_state', {});
        return state.ok && !authStateValue(state).isAuthenticated;
      },
      {
        timeout: 20_000,
        interval: 1_000,
        timeoutMsg: 'core still reports isAuthenticated=true after the backend revoked the session',
      }
    );
    const revokedState = await callOpenhumanRpc<AuthStateResponse>('openhuman.auth_get_state', {});
    expectRpcOk('auth_get_state', revokedState);
    expect(authStateValue(revokedState).credential).toBeUndefined();

    // 3. The shell left the authenticated surface.
    const stillOnHome = await waitForHomePage(5_000);
    expect(stillOnHome).toBeNull();
    console.log('[AuthAccess] Revoked session auto-logout verified');
  });
});
