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
import { type AuthStateResponse, authStateValue } from '../helpers/auth-state';
import { callOpenhumanRpc, expectRpcOk } from '../helpers/core-rpc';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  hasAppChrome,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  navigateToHome,
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

/** Install content-free counters before a native auth expiry can be emitted. */
async function installAuthExpiryDiagnostics(): Promise<boolean> {
  return (await browser.execute(async () => {
    const w = window as typeof window & {
      __TAURI_INTERNALS__?: {
        invoke?: (command: string, args?: unknown) => Promise<unknown>;
        transformCallback?: (callback: () => void) => string | number;
      };
      __openhuman235AuthDiagnostics?: {
        nativeExpiredEvents: number;
        rendererExpiredEvents: number;
        nativeListenerId: string | number;
      };
    };
    const internals = w.__TAURI_INTERNALS__;
    if (!internals?.invoke || !internals.transformCallback) return false;

    const diagnostics = {
      nativeExpiredEvents: 0,
      rendererExpiredEvents: 0,
      nativeListenerId: -1 as string | number,
    };
    const handler = internals.transformCallback(() => {
      diagnostics.nativeExpiredEvents += 1;
    });
    diagnostics.nativeListenerId = (await internals.invoke('plugin:event|listen', {
      event: 'auth://expired',
      target: { kind: 'Any' },
      handler,
    })) as string | number;
    window.addEventListener('openhuman:session-expired', () => {
      diagnostics.rendererExpiredEvents += 1;
    });
    w.__openhuman235AuthDiagnostics = diagnostics;
    return true;
  })) as boolean;
}

async function readAuthExpiryDiagnostics() {
  return await browser.execute(() => {
    const w = window as typeof window & {
      __openhuman235AuthDiagnostics?: {
        nativeExpiredEvents: number;
        rendererExpiredEvents: number;
      };
    };
    return {
      nativeExpiredEvents: w.__openhuman235AuthDiagnostics?.nativeExpiredEvents ?? 0,
      rendererExpiredEvents: w.__openhuman235AuthDiagnostics?.rendererExpiredEvents ?? 0,
      route: window.location.hash,
      rootShellVisible: document.querySelector('[data-testid="root-shell-sidebar"]') !== null,
      welcomeVisible: document.querySelector('[data-testid="welcome-card-tinyhumans"]') !== null,
    };
  });
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
    // The WebView simulation dispatches auth deep links asynchronously. Move
    // off Chat first so a subsequent redirect back to the authenticated chat
    // route proves the handler finished; the persistent app shell alone is
    // already visible before that work completes.
    await navigateViaHash('/settings/account');
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

    await browser.waitUntil(
      async () => (await browser.execute(() => window.location.hash)) === '#/chat',
      {
        timeout: 20_000,
        interval: 250,
        timeoutMsg: 'Timed out waiting for the completed auth handoff to return to Chat',
      }
    );

    const homeText = await waitForHomePage(15_000);
    expect(homeText).not.toBeNull();

    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
    );
    expect(consumeCall).toBeDefined();
    console.log('[AuthAccess] Multi-device token accepted');
  });

  // Account/billing/logout rendering and auth clearing are browser-owned.
  it('revoked session auto-logs out the user', async function () {
    this.timeout(120_000);
    // Login fresh
    clearRequestLog();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await performFullLogin('e2e-revoked-session-token');

    // Set mock to return 401 for user profile requests (revoked session)
    setMockBehavior('session', 'revoked');

    // Trigger a re-auth which will fail with 401
    expect(await installAuthExpiryDiagnostics()).toBe(true);
    await triggerAuthDeepLink('e2e-revoked-check-token');

    // Count a completed response, not merely an attempted request. The mock
    // records status when the response finishes; this proves the revoked
    // session branch actually returned 401 before we inspect logout behavior.
    await browser.waitUntil(
      async () =>
        getRequestLog().some(
          r => r.method === 'GET' && r.url.includes('/auth/me') && r.status === 401
        ),
      {
        timeout: 20_000,
        interval: 250,
        timeoutMsg: 'mock did not finish a GET /auth/me response with status 401',
      }
    );

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
    expect(
      getRequestLog().some(
        r => r.method === 'GET' && r.url.includes('/auth/me') && r.status === 401
      )
    ).toBe(true);

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
    const eventDiagnostics = await readAuthExpiryDiagnostics();
    if (stillOnHome !== null) {
      throw new Error(
        `expected revoked session to leave the authenticated shell; observed ${JSON.stringify({
          stillOnHome,
          coreAuthenticated: authStateValue(revokedState).isAuthenticated,
          coreCredentialPresent: Boolean(authStateValue(revokedState).credential),
          ...eventDiagnostics,
        })}`
      );
    }
    expect(stillOnHome).toBeNull();
    console.log('[AuthAccess] Revoked session auto-logout verified');
  });

  // The counterpart of the revoked case: only a 401 expires the session, so a
  // 403 from `/auth/me` (a plan, WAF or proxy refusal) must leave the user
  // signed in.
  it('a forbidden /auth/me keeps the user signed in', async function () {
    this.timeout(120_000);
    clearRequestLog();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await performFullLogin('e2e-forbidden-session-token');

    setMockBehavior('session', 'forbidden');
    await triggerAuthDeepLink('e2e-forbidden-check-token');

    // 1. The injection landed: a completed GET /auth/me answered 403.
    await browser.waitUntil(
      async () =>
        getRequestLog().some(
          r => r.method === 'GET' && r.url.includes('/auth/me') && r.status === 403
        ),
      {
        timeout: 20_000,
        interval: 250,
        timeoutMsg: 'mock did not finish a GET /auth/me response with status 403',
      }
    );

    // 2. The core keeps the credential and the shell stays on Home. The wait
    //    matches the revoked case's 20 s window, in which a 401 signs out, so
    //    a 403 that still signed out would be caught here.
    await browser.pause(20_000);
    const state = await callOpenhumanRpc<AuthStateResponse>('openhuman.auth_get_state', {});
    expectRpcOk('auth_get_state', state);
    expect(authStateValue(state).isAuthenticated).toBe(true);
    expect(await waitForHomePage(5_000)).not.toBeNull();
    resetMockBehavior();
  });
});
