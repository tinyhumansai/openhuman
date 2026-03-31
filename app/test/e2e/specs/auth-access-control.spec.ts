/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Authentication & Access Control + Billing & Subscriptions.
 *
 * Covers:
 *   1.1    User registration via deep link (verified by before() setup)
 *   1.1.1  Duplicate account handling (re-auth same user)
 *   1.2    Multi-device sessions (second JWT accepted)
 *   3.1.1  Default plan allocation (FREE plan on registration)
 *   3.2.1  Upgrade flow (purchase API call + polling)
 *   3.2.2  Downgrade flow (lower tiers have no Upgrade button)
 *   3.3.1  Subscription creation (active subscription display)
 *   3.3.2  Renewal handling (renewal date display)
 *   3.3.3  Cancellation handling (Stripe portal API call)
 *   1.3    Logout via Settings menu
 *   1.3.1  Revoked session auto-logout
 *
 * Each describe block is standalone — the before() hook performs a full
 * login + onboarding cycle so specs don't depend on each other.
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
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

/**
 * Click a native XCUIElementTypeButton by its label/title attribute.
 * Unlike clickText which matches any element, this targets only buttons.
 * Required when text labels sit next to (but outside) the button bounds.
 */
async function clickNativeButton(text, timeout = 10_000) {
  const selector =
    `//XCUIElementTypeButton[contains(@label, "${text}") or ` + `contains(@title, "${text}")]`;
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `Button "${text}" not found within ${timeout}ms` });

  const location = await el.getLocation();
  const size = await el.getSize();
  const centerX = Math.round(location.x + size.width / 2);
  const centerY = Math.round(location.y + size.height / 2);

  await browser.performActions([
    {
      type: 'pointer',
      id: 'mouse1',
      parameters: { pointerType: 'mouse' },
      actions: [
        { type: 'pointerMove', duration: 10, x: centerX, y: centerY },
        { type: 'pointerDown', button: 0 },
        { type: 'pause', duration: 50 },
        { type: 'pointerUp', button: 0 },
      ],
    },
  ]);
  await browser.releaseActions();
}

/**
 * Poll the mock server request log until a matching request appears.
 */
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

/**
 * Wait until the given text disappears from the accessibility tree.
 */
async function waitForTextToDisappear(text, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await textExists(text))) return true;
    await browser.pause(500);
  }
  return false;
}

/**
 * Wait until one of the candidate texts appears on screen (Home page markers).
 * Returns the matched text or null.
 */
async function waitForHomePage(timeout = 15_000) {
  const candidates = [
    'Test',
    'Good morning',
    'Good afternoon',
    'Good evening',
    'Message OpenHuman',
    'Upgrade to Premium',
  ];

  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(1_000);
  }
  return null;
}

/**
 * Wait until one of the Welcome/public page markers appears.
 * Returns the matched text or null.
 */
async function waitForPublicPage(timeout = 15_000) {
  const candidates = ['Welcome', 'Log in', 'Sign in', 'Get Started', 'openhuman'];

  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(1_000);
  }
  return null;
}

/**
 * Click the first matching text from a list of candidates, with retry.
 * Returns the text that was clicked, or null if none found.
 */
async function clickFirstCandidate(candidates, label, timeout = 10_000) {
  // First attempt
  for (const text of candidates) {
    if (await textExists(text)) {
      await clickText(text, timeout);
      console.log(`[AuthAccess] ${label}: clicked "${text}"`);

      // Verify the click advanced (text should disappear)
      const advanced = await waitForTextToDisappear(text, 8_000);
      if (advanced) return text;

      // If text didn't disappear, retry the click
      console.log(`[AuthAccess] ${label}: "${text}" still visible, retrying click...`);
      await clickText(text, 5_000);
      const retryAdvanced = await waitForTextToDisappear(text, 5_000);
      if (retryAdvanced) return text;

      // Still stuck — log and return null so callers surface the failure
      const tree = await dumpAccessibilityTree();
      console.log(
        `[AuthAccess] ${label}: "${text}" still visible after retry. Tree:\n`,
        tree.slice(0, 4000)
      );
      return null;
    }
  }

  // If no candidate found, dump tree for debugging
  const tree = await dumpAccessibilityTree();
  console.log(`[AuthAccess] ${label}: no candidates found. Tree:\n`, tree.slice(0, 4000));
  return null;
}

/**
 * Navigate to the Billing panel: Settings → Billing & Usage.
 * Returns when billing page content is visible.
 */
async function navigateToBilling() {
  await clickNativeButton('Settings', 10_000);
  console.log('[AuthAccess] Clicked Settings nav');
  await browser.pause(2_000);

  // Click "Billing" or "Billing & Usage" menu item
  const billingCandidates = ['Billing & Usage', 'Billing'];
  let clicked = false;
  for (const text of billingCandidates) {
    if (await textExists(text)) {
      await clickText(text, 10_000);
      console.log(`[AuthAccess] Clicked "${text}" menu item`);
      clicked = true;
      break;
    }
  }
  if (!clicked) {
    const tree = await dumpAccessibilityTree();
    console.log('[AuthAccess] Billing menu item not found. Tree:\n', tree.slice(0, 6000));
    throw new Error('Billing menu item not found in Settings');
  }

  await browser.pause(2_000);
}

/**
 * Navigate from Settings/Billing back to Home via the sidebar Home button.
 */
async function navigateToHome() {
  await clickNativeButton('Home', 10_000);
  console.log('[AuthAccess] Clicked Home nav');
  await browser.pause(2_000);
  const homeText = await waitForHomePage(10_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log('[AuthAccess] navigateToHome: Home page not reached. Tree:\n', tree.slice(0, 4000));
    throw new Error('navigateToHome: Home page not reached after clicking Home nav');
  }
}

/**
 * Perform the full login + onboarding flow via deep link.
 * Leaves the app on the Home page.
 */
async function performFullLogin(token = 'e2e-test-token') {
  await triggerAuthDeepLink(token);

  // Wait for window + WebView + accessibility tree
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
  const meCall = await waitForRequest('GET', '/telegram/me', 20_000);
  if (!meCall) {
    console.log(
      '[AuthAccess] Missing /telegram/me call. Request log:',
      JSON.stringify(getRequestLog(), null, 2)
    );
    throw new Error('/telegram/me call missing in performFullLogin');
  }

  // Onboarding Step 1: InviteCodeStep — skip
  await clickText('Skip for now', 10_000);
  console.log('[AuthAccess] Clicked "Skip for now"');

  const stepChanged = await waitForTextToDisappear('Skip for now', 8_000);
  if (!stepChanged) {
    console.log('[AuthAccess] Step did not advance, retrying...');
    await clickText('Skip', 5_000);
    await waitForTextToDisappear('Skip', 5_000);
  }
  await browser.pause(2_000);

  // Onboarding Step 2: FeaturesStep
  const featResult = await clickFirstCandidate(['Looks Amazing', 'Bring It On'], 'FeaturesStep');
  if (!featResult) throw new Error('FeaturesStep button not found');
  await browser.pause(2_000);

  // Onboarding Step 3: PrivacyStep
  const privResult = await clickFirstCandidate(['Got it', 'Continue'], 'PrivacyStep');
  if (!privResult) throw new Error('PrivacyStep button not found');
  await browser.pause(2_000);

  // Onboarding Step 4: GetStartedStep
  const startResult = await clickFirstCandidate(["Let's Go", "I'm Ready"], 'GetStartedStep');
  if (!startResult) throw new Error('GetStartedStep button not found');
  await browser.pause(3_000);

  // Verify we landed on Home
  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(
      '[AuthAccess] Home page not reached after onboarding. Tree:\n',
      tree.slice(0, 4000)
    );
    throw new Error('Full login + onboarding did not reach Home page');
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
    clearRequestLog();

    // Perform full login + onboarding so subsequent tests start from Home
    await performFullLogin('e2e-auth-token');
  });

  after(async function () {
    this.timeout(30_000);
    resetMockBehavior();
    try {
      await stopMockServer();
    } catch (err) {
      console.log('[AuthAccess] stopMockServer error (non-fatal):', err);
    }
  });

  // -------------------------------------------------------------------------
  // 1.1 User Registration
  // -------------------------------------------------------------------------

  it('new user registers via deep link and reaches home', async () => {
    // This was already verified by before() — just assert the API calls
    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/telegram/login-tokens/')
    );
    expect(consumeCall).toBeDefined();

    const meCall = getRequestLog().find(r => r.method === 'GET' && r.url.includes('/telegram/me'));
    expect(meCall).toBeDefined();

    // Confirm we're on Home
    const homeText = await waitForHomePage(5_000);
    expect(homeText).not.toBeNull();
  });

  // -------------------------------------------------------------------------
  // 1.1.1 Duplicate Account Handling
  // -------------------------------------------------------------------------

  it('re-authenticating with a new token for the same user returns to home', async () => {
    clearRequestLog();

    // Trigger a second deep link — backend returns the same MOCK_USER
    await triggerAuthDeepLink('e2e-dup-token');
    await browser.pause(5_000);

    // App should process the token and stay on / return to Home
    // (already onboarded, so ProtectedRoute sends to /home)
    const consumeCall = await waitForRequest('POST', '/telegram/login-tokens/', 10_000);
    if (!consumeCall) {
      console.log('[AuthAccess] Dup request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(consumeCall).toBeDefined();

    const homeText = await waitForHomePage(10_000);
    if (!homeText) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] Dup: not on Home. Tree:\n', tree.slice(0, 4000));
    }
    expect(homeText).not.toBeNull();
  });

  // -------------------------------------------------------------------------
  // 1.2 Multi-Device Sessions
  // -------------------------------------------------------------------------

  it('second device token is accepted and processed', async () => {
    clearRequestLog();

    // Mock returns a different JWT for "device 2"
    setMockBehavior('jwt', 'device2');

    await triggerAuthDeepLink('e2e-device2-token');
    await browser.pause(5_000);

    const consumeCall = await waitForRequest('POST', '/telegram/login-tokens/', 10_000);
    if (!consumeCall) {
      console.log('[AuthAccess] Device2 request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(consumeCall).toBeDefined();

    // App should land on Home (already onboarded) or onboarding
    const homeText = await waitForHomePage(10_000);
    expect(homeText).not.toBeNull();

    // Reset for next tests
    resetMockBehavior();
  });

  // -------------------------------------------------------------------------
  // 3.1 Plan Assignment
  // -------------------------------------------------------------------------

  it('3.1.1 — new user is assigned FREE plan by default', async () => {
    // Navigate to Settings → Billing & Usage
    await navigateToBilling();

    // Verify billing page loaded with plan info
    const hasPlanText = await textExists('Your Current Plan');
    if (!hasPlanText) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] Billing page tree:\n', tree.slice(0, 6000));
    }
    expect(hasPlanText).toBe(true);

    // Verify FREE plan is shown
    const hasFree = await textExists('FREE');
    expect(hasFree).toBe(true);

    // Verify "Current" badge is displayed (next to the Free tier card)
    const hasCurrent = await textExists('Current');
    expect(hasCurrent).toBe(true);

    // Verify "Upgrade" button exists (for BASIC and/or PRO tiers)
    const hasUpgrade = await textExists('Upgrade');
    expect(hasUpgrade).toBe(true);

    console.log('[AuthAccess] 3.1.1 — FREE plan verified in billing');

    // Navigate back to Home for next tests
    await navigateToHome();
  });

  // -------------------------------------------------------------------------
  // 3.2 Plan Changes
  // -------------------------------------------------------------------------

  it('3.2.1 — upgrade initiates purchase flow via Stripe', async () => {
    // Navigate to billing
    await navigateToBilling();
    clearRequestLog();

    // Click the first "Upgrade" button (BASIC tier, appears before PRO)
    await clickText('Upgrade', 10_000);
    console.log('[AuthAccess] Clicked Upgrade button');
    await browser.pause(3_000);

    // Verify POST /payments/stripe/purchasePlan was called
    const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
    if (!purchaseCall) {
      console.log('[AuthAccess] Purchase request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(purchaseCall).toBeDefined();

    // Verify the request body contains a BASIC plan identifier
    if (purchaseCall?.body) {
      const bodyStr = typeof purchaseCall.body === 'string' ? purchaseCall.body : '';
      console.log('[AuthAccess] Purchase request body:', bodyStr);
      expect(bodyStr).toContain('BASIC');
    }

    // Verify the app entered purchasing state ("Waiting..." button text)
    const hasWaiting =
      (await textExists('Waiting')) || (await textExists('Waiting for payment confirmation'));
    console.log(`[AuthAccess] Purchasing state visible: ${hasWaiting}`);
    expect(hasWaiting).toBe(true);

    // Switch mock to BASIC plan so polling succeeds
    setMockBehavior('plan', 'BASIC');
    setMockBehavior('planActive', 'true');
    setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());

    // Wait for polling to detect the plan change (polls every 5s, give it 20s)
    const waitingGone = await waitForTextToDisappear('Waiting', 20_000);
    console.log(`[AuthAccess] Purchasing state cleared: ${waitingGone}`);

    console.log('[AuthAccess] 3.2.1 — Upgrade purchase flow verified');

    // Navigate back to Home
    await navigateToHome();
  });

  it('3.2.2 — lower tier does not show Upgrade button (downgrade not available)', async () => {
    // Mock is already set to BASIC plan from previous test.
    // Trigger deep link re-auth to refresh team state with BASIC subscription.
    clearRequestLog();
    await triggerAuthDeepLink('e2e-billing-refresh-token');
    await browser.pause(5_000);

    // Wait for the re-auth to complete and reach Home
    const homeText = await waitForHomePage(15_000);
    expect(homeText).not.toBeNull();
    console.log('[AuthAccess] Re-authed with BASIC plan, on Home');

    // Navigate to billing
    await navigateToBilling();

    // Verify BASIC is the current plan
    const hasBasic = (await textExists('BASIC')) || (await textExists('Basic'));
    expect(hasBasic).toBe(true);

    // Verify "Current" badge is visible (next to BASIC tier)
    const hasCurrent = await textExists('Current');
    expect(hasCurrent).toBe(true);

    // Count all elements containing "Upgrade" — only PRO should have the button.
    // Free is a downgrade from BASIC so isUpgrade() returns false → no Upgrade button.
    const upgradeSelector = `//*[contains(@label, "Upgrade") or contains(@value, "Upgrade") or contains(@title, "Upgrade")]`;
    const upgradeElements = await browser.$$(upgradeSelector);
    const upgradeCount = upgradeElements.length;
    console.log(`[AuthAccess] Found ${upgradeCount} "Upgrade" element(s)`);
    expect(upgradeCount).toBe(1);

    // Verify PRO plan is visible — the only tier above BASIC that should show Upgrade
    const hasPro = (await textExists('PRO')) || (await textExists('Pro'));
    expect(hasPro).toBe(true);
    console.log('[AuthAccess] 3.2.2 — Exactly 1 Upgrade (PRO only), downgrade not offered');

    // Stay on billing for the next subscription lifecycle tests
  });

  // -------------------------------------------------------------------------
  // 3.3 Subscription Lifecycle
  // -------------------------------------------------------------------------

  it('3.3.1 — active subscription is displayed correctly', async () => {
    // Still on billing page from previous test with BASIC plan active.
    // Verify subscription indicators are visible.

    // The mock has planActive='true', so "Manage Subscription" should appear
    const hasManage = await textExists('Manage Subscription');
    if (!hasManage) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] Manage Subscription not found. Tree:\n', tree.slice(0, 6000));
    }
    expect(hasManage).toBe(true);

    // Verify the current plan API was called (BillingPanel fetches on mount)
    const planCall = getRequestLog().find(
      r => r.method === 'GET' && r.url.includes('/payments/stripe/currentPlan')
    );
    expect(planCall).toBeDefined();

    console.log('[AuthAccess] 3.3.1 — Active subscription display verified');
  });

  it('3.3.2 — renewal date is displayed for active subscription', async () => {
    // Still on billing page with BASIC plan active and planExpiry set.
    // The mock has planExpiry set to ~30 days from now.
    const hasRenews = await textExists('Renews');
    if (!hasRenews) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] Renews text not found. Tree:\n', tree.slice(0, 6000));
    }
    expect(hasRenews).toBe(true);

    console.log('[AuthAccess] 3.3.2 — Renewal date display verified');
  });

  it('3.3.3 — manage subscription opens Stripe portal', async () => {
    // Still on billing page with active subscription.
    clearRequestLog();

    // Click "Manage Subscription"
    await clickText('Manage Subscription', 10_000);
    console.log('[AuthAccess] Clicked Manage Subscription');
    await browser.pause(3_000);

    // Verify POST /payments/stripe/portal was called
    const portalCall = await waitForRequest('POST', '/payments/stripe/portal', 10_000);
    if (!portalCall) {
      console.log('[AuthAccess] Portal request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(portalCall).toBeDefined();

    console.log('[AuthAccess] 3.3.3 — Stripe portal API call verified');

    // Reset billing mock behavior and navigate back to Home for logout tests
    resetMockBehavior();
    await navigateToHome();

    // Re-auth to restore clean state (FREE plan) for logout tests
    clearRequestLog();
    await triggerAuthDeepLink('e2e-pre-logout-token');
    await browser.pause(5_000);
    const homeAfterReset = await waitForHomePage(15_000);
    expect(homeAfterReset).not.toBeNull();
    console.log('[AuthAccess] Restored clean state for logout tests');
  });

  // -------------------------------------------------------------------------
  // 1.3 Logout & Revocation
  // -------------------------------------------------------------------------

  it('user can log out via Settings and returns to Welcome', async () => {
    // Open Settings — must click the actual Button element, not the text label
    // next to it (they have separate bounding boxes in the sidebar).
    await clickNativeButton('Settings', 10_000);
    console.log('[AuthAccess] Clicked Settings button');
    await browser.pause(3_000);

    // Verify we navigated to the Settings page
    const settingsTree = await dumpAccessibilityTree();
    console.log('[AuthAccess] Settings page tree:\n', settingsTree.slice(0, 6000));

    // Look for "Log out" or related text — it's a <button> element in the
    // settings page with title "Log out" rendered as text inside it.
    const logoutCandidates = ['Log out', 'Sign out', 'Logout', 'log out'];
    let logoutFound = false;

    for (const text of logoutCandidates) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[AuthAccess] Clicked logout: "${text}"`);
        logoutFound = true;
        break;
      }
    }

    if (!logoutFound) {
      // Settings page may need scrolling — "Log out" is at the bottom.
      // Use mouse wheel scroll inside the content area.
      console.log('[AuthAccess] Log out not visible, attempting scroll...');
      try {
        const webView = await browser.$('//XCUIElementTypeWebView');
        const loc = await webView.getLocation();
        const size = await webView.getSize();
        // Scroll inside the right content area (not the sidebar)
        const scrollX = Math.round(loc.x + size.width * 0.65);
        const startY = Math.round(loc.y + size.height * 0.8);
        const endY = Math.round(loc.y + size.height * 0.2);

        await browser.performActions([
          {
            type: 'pointer',
            id: 'scroll1',
            parameters: { pointerType: 'mouse' },
            actions: [
              { type: 'pointerMove', duration: 10, x: scrollX, y: startY },
              { type: 'pointerDown', button: 0 },
              { type: 'pointerMove', duration: 300, x: scrollX, y: endY },
              { type: 'pointerUp', button: 0 },
            ],
          },
        ]);
        await browser.releaseActions();
        await browser.pause(2_000);
      } catch (scrollErr) {
        console.log('[AuthAccess] Scroll attempt failed:', scrollErr);
      }

      // Dump tree after scroll to see what's now visible
      const afterScrollTree = await dumpAccessibilityTree();
      console.log('[AuthAccess] After scroll tree:\n', afterScrollTree.slice(0, 6000));

      // Try again after scroll
      for (const text of logoutCandidates) {
        if (await textExists(text)) {
          await clickText(text, 5_000);
          console.log(`[AuthAccess] Clicked logout after scroll: "${text}"`);
          logoutFound = true;
          break;
        }
      }
    }

    if (!logoutFound) {
      throw new Error('Could not find logout button in Settings');
    }

    await browser.pause(3_000);

    // Should land on the public Welcome page
    const publicText = await waitForPublicPage(15_000);
    if (!publicText) {
      const tree = await dumpAccessibilityTree();
      console.log('[AuthAccess] After logout, not on public page. Tree:\n', tree.slice(0, 4000));
    }
    expect(publicText).not.toBeNull();
    console.log(`[AuthAccess] Post-logout public page confirmed: found "${publicText}"`);

    // Verify no protected content is visible
    const homeStillVisible = await textExists('Message OpenHuman');
    expect(homeStillVisible).toBe(false);
  });

  it('revoked session auto-logs out the user', async () => {
    // After the previous test logged out, we're on the public page.
    // Set mock to return 401 on /telegram/me (session revoked)
    setMockBehavior('session', 'revoked');
    clearRequestLog();

    // Login again via deep link — token consume succeeds, but /telegram/me returns 401
    await triggerAuthDeepLink('e2e-revoked-token');
    await browser.pause(5_000);

    // The app gets a JWT, navigates to onboarding/home, UserProvider calls
    // /telegram/me -> 401 -> clearToken() -> redirect to Welcome
    const consumeCall = await waitForRequest('POST', '/telegram/login-tokens/', 10_000);
    expect(consumeCall).toBeDefined();

    // Wait for the auto-logout cycle: login -> fetch user -> 401 -> clearToken -> public page
    await browser.pause(5_000);

    const publicText = await waitForPublicPage(15_000);
    if (!publicText) {
      const tree = await dumpAccessibilityTree();
      console.log(
        '[AuthAccess] After revocation, not on public page. Tree:\n',
        tree.slice(0, 4000)
      );
    }
    expect(publicText).not.toBeNull();
    console.log(`[AuthAccess] Revoked session auto-logout confirmed: found "${publicText}"`);

    resetMockBehavior();
  });
});
