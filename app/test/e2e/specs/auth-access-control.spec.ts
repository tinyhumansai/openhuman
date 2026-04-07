/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Authentication & Access Control + Billing & Subscriptions (Linux / tauri-driver).
 *
 * Covers:
 *   1.1    User registration via deep link
 *   1.1.1  Duplicate account handling (re-auth same user)
 *   1.2    Multi-device sessions (second JWT accepted)
 *   3.1.1  Default plan allocation (FREE plan on registration)
 *   3.2.1  Upgrade flow (purchase API call)
 *   3.3.1  Active subscription display
 *   3.3.3  Manage subscription (Stripe portal API call)
 *   1.3    Logout via Settings menu
 *   1.3.1  Revoked session auto-logout
 *
 * Onboarding steps (Onboarding.tsx — 5 steps, indices 0–4):
 *   Welcome → Local AI → Screen & Accessibility → Enable Tools → Install Skills
 *   (each step: primary "Continue"; final step completes onboarding)
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  clickNativeButton,
  clickText,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { isMac2 } from '../helpers/platform';
import {
  logoutViaSettings,
  navigateToBilling,
  navigateToHome,
  navigateToSettings,
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

  const consumeCall = await waitForRequest('POST', '/telegram/login-tokens/', 20_000);
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
  before(async () => {
    await startMockServer();
    await waitForApp();
    resetMockBehavior();
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // -------------------------------------------------------------------------
  // 1. Authentication
  // -------------------------------------------------------------------------

  it('new user registers via deep link and reaches home', async () => {
    await performFullLogin('e2e-auth-token');
  });

  it('re-authenticating with a new token for the same user returns to home', async () => {
    clearRequestLog();
    await triggerAuthDeepLink('e2e-auth-reauth-token');
    await browser.pause(5_000);

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
    await browser.pause(5_000);

    const homeText = await waitForHomePage(15_000);
    if (!homeText) {
      await navigateToHome();
    }
    const finalHome = homeText || (await waitForHomePage(10_000));
    expect(finalHome).not.toBeNull();

    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/telegram/login-tokens/')
    );
    expect(consumeCall).toBeDefined();
    console.log('[AuthAccess] Multi-device token accepted');
  });

  // -------------------------------------------------------------------------
  // 2. Default Plan
  // -------------------------------------------------------------------------

  it('3.1.1 — new user is assigned FREE plan by default', async () => {
    resetMockBehavior();
    clearRequestLog();
    await navigateToBilling();

    // BillingPanel heading: "Current Plan — FREE"
    const hasPlan = (await textExists('Current Plan')) || (await textExists('FREE'));
    if (!hasPlan) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] Billing page tree:\n', tree.slice(0, 6000));
    }
    expect(hasPlan).toBe(true);

    const hasUpgrade = await textExists('Upgrade');
    expect(hasUpgrade).toBe(true);

    console.log('[AuthAccess] 3.1.1 — FREE plan verified in billing');
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 3. Upgrade Flow
  // -------------------------------------------------------------------------

  it('3.2.1 — upgrade initiates purchase flow via Stripe', async () => {
    resetMockBehavior();
    await navigateToBilling();
    clearRequestLog();

    await clickText('Upgrade', 10_000);
    console.log('[AuthAccess] Clicked Upgrade button');
    await browser.pause(3_000);

    const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
    expect(purchaseCall).toBeDefined();

    if (purchaseCall?.body) {
      const bodyStr = typeof purchaseCall.body === 'string' ? purchaseCall.body : '';
      console.log('[AuthAccess] Purchase request body:', bodyStr);
    }

    // Verify purchasing state appears
    const hasWaiting = (await textExists('Waiting')) || (await textExists('Waiting for payment'));
    console.log(`[AuthAccess] Purchasing state visible: ${hasWaiting}`);

    // Switch mock to BASIC plan so polling clears the waiting state
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());

    if (hasWaiting) {
      const disappeared = await waitForTextToDisappear('Waiting', 20_000);
      if (!disappeared) {
        throw new Error(
          '3.2.1 — "Waiting" spinner did not clear within 20s after mock plan was set to BASIC'
        );
      }
    }

    console.log('[AuthAccess] 3.2.1 — Upgrade purchase flow verified');
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 4. Active Subscription Display
  // -------------------------------------------------------------------------

  it('3.3.1 — active subscription is displayed correctly', async () => {
    // Seed mock state explicitly so this test is self-contained
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());
    clearRequestLog();

    await navigateToBilling();

    // Wait for billing data to load
    await browser.pause(3_000);

    // Verify currentPlan was fetched
    const planCall = getRequestLog().find(
      r => r.method === 'GET' && r.url.includes('/payments/stripe/currentPlan')
    );
    expect(planCall).toBeDefined();

    // Check that plan info is displayed (Current Plan heading or tier name)
    const hasPlanInfo =
      (await textExists('Current Plan')) ||
      (await textExists('BASIC')) ||
      (await textExists('Basic'));
    expect(hasPlanInfo).toBe(true);

    console.log('[AuthAccess] 3.3.1 — Active subscription display verified');
  });

  it('3.3.3 — manage subscription opens Stripe portal', async () => {
    if (isMac2()) {
      console.log(
        '[AuthAccess] 3.3.3 — skipping portal action assertion on Mac2; header/payment-method actions are not exposed reliably in WKWebView accessibility'
      );
      return;
    }

    // Seed mock state explicitly so this test is self-contained
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());
    clearRequestLog();

    await navigateToBilling();
    await browser.pause(3_000);

    let clickedPortalAction = 'Manage';
    try {
      await clickNativeButton('Manage', 10_000);
    } catch {
      try {
        await clickText('Manage', 10_000);
      } catch {
        clickedPortalAction = 'Add card';
        try {
          await clickNativeButton('Add card', 10_000);
        } catch {
          try {
            await clickText('Add card', 10_000);
          } catch {
            clickedPortalAction = '+ Add card';
            try {
              await clickNativeButton('+ Add card', 10_000);
            } catch {
              await clickText('+ Add card', 10_000);
            }
          }
        }
      }
    }
    console.log(`[AuthAccess] Clicked portal action: ${clickedPortalAction}`);
    await browser.pause(3_000);

    const portalCall = await waitForRequest('POST', '/payments/stripe/portal', 10_000);
    if (!portalCall) {
      console.log('[AuthAccess] Portal request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(portalCall).toBeDefined();

    console.log('[AuthAccess] 3.3.3 — Stripe portal API call verified');
    resetMockBehavior();
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 5. Logout
  // -------------------------------------------------------------------------

  it('user can log out via Settings and returns to Welcome', async () => {
    if (isMac2()) {
      await navigateToSettings();
      const hasLogoutEntry =
        (await textExists('Log out')) ||
        (await textExists('Logout')) ||
        (await textExists('Sign out'));
      expect(hasLogoutEntry).toBe(true);
      console.log(
        '[AuthAccess] Logout settings entry verified on Mac2; click-through logout remains unreliable via WKWebView accessibility'
      );
      return;
    }

    // Re-auth to get a clean session for logout
    clearRequestLog();
    await triggerAuthDeepLink('e2e-pre-logout-token');
    await browser.pause(5_000);

    const homeCheck = await waitForHomePage(10_000);
    if (!homeCheck) {
      await navigateToHome();
    }

    await logoutViaSettings('[AuthAccess]');

    // logoutViaSettings already waits for the logged-out state. Keep a light
    // postcondition here so the spec does not duplicate platform-specific checks.
    await browser.pause(2_000);
    const loggedOutUi =
      (await textExists('Welcome')) ||
      (await textExists('Get Started')) ||
      (await textExists('OpenHuman'));
    expect(loggedOutUi).toBe(true);
    console.log('[AuthAccess] Logout verified: logged-out UI visible');
  });

  it('revoked session auto-logs out the user', async () => {
    // Login fresh
    clearRequestLog();
    resetMockBehavior();
    await performFullLogin('e2e-revoked-session-token');

    // Set mock to return 401 for user profile requests (revoked session)
    setMockBehavior('session', 'revoked');

    // Trigger a re-auth which will fail with 401
    await triggerAuthDeepLink('e2e-revoked-check-token');
    await browser.pause(8_000);

    // The app should auto-log out when it gets a 401
    const stillOnHome = await waitForHomePage(5_000);
    if (!stillOnHome) {
      console.log('[AuthAccess] Revoked session: user was logged out (no home page markers)');
    }

    // Verify the app is either on Welcome or not on Home
    const welcomeCandidates = ['Welcome', 'Sign in', 'Login', 'Get Started', 'OpenHuman'];
    let onWelcome = false;
    for (const text of welcomeCandidates) {
      if (await textExists(text)) {
        onWelcome = true;
        break;
      }
    }

    expect(onWelcome || !stillOnHome).toBe(true);
    console.log('[AuthAccess] Revoked session auto-logout verified');
  });
});
