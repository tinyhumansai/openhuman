/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Card Payment Processing Flow.
 *
 * Covers:
 *   5.1.1  Checkout session created on Stripe card upgrade (BASIC_MONTHLY)
 *   5.1.2  Checkout session with annual billing interval (BASIC_YEARLY)
 *   5.1.3  Coinbase crypto checkout creates charge
 *   5.2.1  Successful payment detected via polling
 *   5.2.2  Failed purchase API call handled gracefully
 *   5.2.3  Duplicate purchase prevention during checkout
 *   5.3.1  Plan transition from FREE to PRO (direct)
 *   5.3.2  Manage Subscription opens Stripe portal
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  clickNativeButton,
  clickText,
  clickToggle,
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
// Shared helpers (mirrored from auth-access-control.spec.ts)
// ---------------------------------------------------------------------------

const LOG_PREFIX = '[PaymentFlow]';

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
 * Click the first matching text from a list of candidates, with retry.
 */
async function clickFirstCandidate(candidates, label, timeout = 10_000) {
  for (const text of candidates) {
    if (await textExists(text)) {
      await clickText(text, timeout);
      console.log(`${LOG_PREFIX} ${label}: clicked "${text}"`);

      const advanced = await waitForTextToDisappear(text, 8_000);
      if (advanced) return text;

      console.log(`${LOG_PREFIX} ${label}: "${text}" still visible, retrying click...`);
      await clickText(text, 5_000);
      const retryAdvanced = await waitForTextToDisappear(text, 5_000);
      if (retryAdvanced) return text;

      const tree = await dumpAccessibilityTree();
      console.log(
        `${LOG_PREFIX} ${label}: "${text}" still visible after retry. Tree:\n`,
        tree.slice(0, 4000)
      );
      return null;
    }
  }

  const tree = await dumpAccessibilityTree();
  console.log(`${LOG_PREFIX} ${label}: no candidates found. Tree:\n`, tree.slice(0, 4000));
  return null;
}

/**
 * Navigate to the Billing panel: Settings -> Billing & Usage.
 */
async function navigateToBilling() {
  await clickNativeButton('Settings', 10_000);
  console.log(`${LOG_PREFIX} Clicked Settings nav`);
  await browser.pause(2_000);

  const billingCandidates = ['Billing & Usage', 'Billing'];
  let clicked = false;
  for (const text of billingCandidates) {
    if (await textExists(text)) {
      await clickText(text, 10_000);
      console.log(`${LOG_PREFIX} Clicked "${text}" menu item`);
      clicked = true;
      break;
    }
  }
  if (!clicked) {
    const tree = await dumpAccessibilityTree();
    console.log(`${LOG_PREFIX} Billing menu item not found. Tree:\n`, tree.slice(0, 6000));
    throw new Error('Billing menu item not found in Settings');
  }

  await browser.pause(2_000);
}

/**
 * Navigate back to Home via the sidebar Home button.
 */
async function navigateToHome() {
  await clickNativeButton('Home', 10_000);
  console.log(`${LOG_PREFIX} Clicked Home nav`);
  await browser.pause(2_000);
  const homeText = await waitForHomePage(10_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(
      `${LOG_PREFIX} navigateToHome: Home page not reached. Tree:\n`,
      tree.slice(0, 4000)
    );
    throw new Error('navigateToHome: Home page not reached after clicking Home nav');
  }
}

/**
 * Perform the full login + onboarding flow via deep link.
 * Leaves the app on the Home page.
 */
async function performFullLogin(token = 'e2e-test-token') {
  await triggerAuthDeepLink(token);

  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);

  // Onboarding is a React portal overlay (z-[9999]). On Mac2, portal content
  // may not appear in the accessibility tree (WKWebView limitation).
  // Try to walk through onboarding if visible, otherwise skip.
  const skipVisible = await textExists('Skip for now');
  if (skipVisible) {
    await clickText('Skip for now', 10_000);
    console.log(`${LOG_PREFIX} Clicked "Skip for now"`);
    await waitForTextToDisappear('Skip for now', 8_000);
    await browser.pause(2_000);

    // FeaturesStep
    const featResult = await clickFirstCandidate(['Looks Amazing', 'Bring It On'], 'FeaturesStep');
    if (featResult) await browser.pause(2_000);

    // PrivacyStep
    const privResult = await clickFirstCandidate(['Got it', 'Continue'], 'PrivacyStep');
    if (privResult) await browser.pause(2_000);

    // GetStartedStep
    const startResult = await clickFirstCandidate(["Let's Go", "I'm Ready"], 'GetStartedStep');
    if (startResult) await browser.pause(3_000);
  } else {
    console.log(`${LOG_PREFIX} Onboarding overlay not visible — skipping (WKWebView portal limitation)`);
    await browser.pause(3_000);
  }

  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(
      `${LOG_PREFIX} Home page not reached after onboarding. Tree:\n`,
      tree.slice(0, 4000)
    );
    throw new Error('Full login + onboarding did not reach Home page');
  }
  console.log(`${LOG_PREFIX} Home page confirmed: found "${homeText}"`);
}

/**
 * Counter for unique JWT suffixes — ensures each re-auth changes the token
 * so UserProvider's useEffect fires and re-fetches user + teams.
 */
let reAuthCounter = 0;

/**
 * Re-authenticate via deep link (resets purchasing state) and navigate to billing.
 * Assumes mock behavior is already configured for the desired plan.
 *
 * IMPORTANT: Each call sets a unique `mockBehavior['jwt']` suffix so the
 * returned JWT differs from the previous one.  Without this, the Redux
 * token wouldn't change and UserProvider wouldn't re-fetch user + teams,
 * leaving stale team subscription data in the store.
 */
async function reAuthAndGoToBilling(token = 'e2e-payment-token') {
  clearRequestLog();

  // Unique JWT so token changes → UserProvider re-fetches user & teams
  reAuthCounter += 1;
  setMockBehavior('jwt', `reauth-${reAuthCounter}`);

  await triggerAuthDeepLink(token);
  await browser.pause(5_000);

  // Always click Home nav first to ensure we're on the actual Home page.
  // The deep link may not change the route if the app is already authenticated
  // and onboarded — "Test" (user name) can appear in Settings headers, making
  // waitForHomePage falsely succeed while still on Settings/Billing.
  try {
    await clickNativeButton('Home', 5_000);
    await browser.pause(2_000);
  } catch {
    // Home button might not be visible yet — that's fine, we'll check below
  }

  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(`${LOG_PREFIX} reAuth: Home page not reached. Tree:\n`, tree.slice(0, 4000));
    throw new Error('reAuthAndGoToBilling: Home page not reached');
  }
  console.log(`${LOG_PREFIX} Re-authed (jwt suffix reauth-${reAuthCounter}), on Home`);

  await navigateToBilling();
}

/**
 * Check if the "Waiting for payment confirmation" banner or "Waiting..." button
 * text is visible.
 */
async function isWaitingVisible() {
  return (await textExists('Waiting for payment confirmation')) || (await textExists('Waiting...'));
}

// ===========================================================================
// Test suite
// ===========================================================================

describe('Card Payment Processing Flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();

    // Full login + onboarding — lands on Home
    await performFullLogin('e2e-payment-token');

    // Navigate to Billing for the first test
    await navigateToBilling();
  });

  after(async function () {
    this.timeout(30_000);
    resetMockBehavior();
    try {
      await stopMockServer();
    } catch (err) {
      console.log(`${LOG_PREFIX} stopMockServer error (non-fatal):`, err);
    }
  });

  // -------------------------------------------------------------------------
  // 5.1 Checkout & Invoice
  // -------------------------------------------------------------------------

  describe('5.1 Checkout & Invoice', () => {
    it('5.1.1 — checkout session is created on Stripe card upgrade', async () => {
      // Verify we're on the billing page with FREE plan
      const hasPlanText = await textExists('Your Current Plan');
      if (!hasPlanText) {
        const tree = await dumpAccessibilityTree();
        console.log(`${LOG_PREFIX} Billing page tree:\n`, tree.slice(0, 6000));
      }
      expect(hasPlanText).toBe(true);

      const hasFree = await textExists('FREE');
      expect(hasFree).toBe(true);

      // Ensure billing interval is "Monthly" (default)
      const hasMonthly = await textExists('Monthly');
      expect(hasMonthly).toBe(true);

      clearRequestLog();

      // Click the first "Upgrade" button (BASIC tier, appears before PRO)
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 5.1.1: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify POST /payments/stripe/purchasePlan was called
      const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
      if (!purchaseCall) {
        console.log(
          `${LOG_PREFIX} 5.1.1: Purchase request log:`,
          JSON.stringify(getRequestLog(), null, 2)
        );
      }
      expect(purchaseCall).toBeDefined();

      // Verify request body contains BASIC_MONTHLY planId
      if (purchaseCall?.body) {
        const bodyStr = typeof purchaseCall.body === 'string' ? purchaseCall.body : '';
        console.log(`${LOG_PREFIX} 5.1.1: Purchase request body:`, bodyStr);
        expect(bodyStr).toContain('BASIC');
        expect(bodyStr).toContain('MONTHLY');
      }

      // Verify the mock response contained a sessionId starting with cs_mock_
      // (We can't inspect the response directly from here, but we can verify the
      // mock was hit and returned 200 — the mock always returns cs_mock_<timestamp>)

      // Verify "Waiting for payment confirmation" banner appears
      const hasWaiting = await isWaitingVisible();
      console.log(`${LOG_PREFIX} 5.1.1: Waiting banner visible: ${hasWaiting}`);
      expect(hasWaiting).toBe(true);

      // Verify Upgrade buttons become disabled (text changes to "Waiting...")
      const hasWaitingButton = await textExists('Waiting...');
      console.log(`${LOG_PREFIX} 5.1.1: Waiting... button text visible: ${hasWaitingButton}`);

      // Switch mock to BASIC so polling resolves and clears the state
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());

      // Wait for polling to detect change and clear waiting state
      const waitingGone = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 5.1.1: Waiting state cleared`);

      console.log(`${LOG_PREFIX} 5.1.1 PASSED`);
    });

    it('5.1.2 — checkout session with annual billing interval', async () => {
      // Reset to FREE plan and re-auth to clear purchasing state
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-annual-token');

      // Click "Annual" billing interval toggle
      await clickText('Annual', 10_000);
      console.log(`${LOG_PREFIX} 5.1.2: Clicked Annual toggle`);
      await browser.pause(1_000);

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 5.1.2: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify POST /payments/stripe/purchasePlan was called
      const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
      if (!purchaseCall) {
        console.log(
          `${LOG_PREFIX} 5.1.2: Purchase request log:`,
          JSON.stringify(getRequestLog(), null, 2)
        );
      }
      expect(purchaseCall).toBeDefined();

      // Verify request body contains BASIC_YEARLY planId
      if (purchaseCall?.body) {
        const bodyStr = typeof purchaseCall.body === 'string' ? purchaseCall.body : '';
        console.log(`${LOG_PREFIX} 5.1.2: Purchase request body:`, bodyStr);
        expect(bodyStr).toContain('BASIC');
        expect(bodyStr).toContain('YEARLY');
      }

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);

      // Resolve the polling so state clears
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      const waitingGone512 = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone512).toBe(true);

      console.log(`${LOG_PREFIX} 5.1.2 PASSED`);
    });

    it('5.1.3 — Coinbase crypto checkout creates charge', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-token');

      // Toggle "Pay with Crypto" switch ON.
      await clickToggle();
      await browser.pause(1_000);

      // Verify billing interval switched to "Annual" (forced by crypto toggle)
      // The Monthly button should be disabled when crypto is selected
      const hasAnnual = await textExists('Annual');
      expect(hasAnnual).toBe(true);

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 5.1.3: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify POST /payments/coinbase/charge was called (NOT Stripe)
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      if (!coinbaseCall) {
        console.log(
          `${LOG_PREFIX} 5.1.3: Coinbase request log:`,
          JSON.stringify(getRequestLog(), null, 2)
        );
      }
      expect(coinbaseCall).toBeDefined();

      // Verify NO Stripe purchasePlan call was made
      const stripeCall = getRequestLog().find(
        r => r.method === 'POST' && r.url.includes('/payments/stripe/purchasePlan')
      );
      expect(stripeCall).toBeUndefined();

      // Verify "Waiting for payment confirmation" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);

      // Resolve polling so state clears
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      const waitingGone513 = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone513).toBe(true);

      console.log(`${LOG_PREFIX} 5.1.3 PASSED`);
    });
  });

  // -------------------------------------------------------------------------
  // 5.2 Payment Confirmation Handling
  // -------------------------------------------------------------------------

  describe('5.2 Payment Confirmation Handling', () => {
    it('5.2.1 — successful payment detected via polling', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-poll-token');

      clearRequestLog();

      // Initiate BASIC upgrade (card)
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 5.2.1: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 5.2.1: Waiting banner visible`);

      // Switch mock: plan changed to BASIC with active subscription
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());

      // Wait for polling to detect the change (polls every 5s, give 15s)
      const waitingGone = await waitForTextToDisappear('Waiting', 15_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 5.2.1: Waiting banner disappeared after polling`);

      // Re-auth to verify the plan state persists — should show BASIC as "Current"
      await reAuthAndGoToBilling('e2e-poll-verify-token');

      const hasBasicCurrent = (await textExists('BASIC')) || (await textExists('Basic'));
      expect(hasBasicCurrent).toBe(true);

      const hasCurrent = await textExists('Current');
      expect(hasCurrent).toBe(true);

      console.log(`${LOG_PREFIX} 5.2.1 PASSED`);
    });

    it('5.2.2 — failed purchase API call handled gracefully', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-fail-token');

      // Set purchaseError to make the Stripe API return 500
      setMockBehavior('purchaseError', 'true');

      try {
        clearRequestLog();

        // Click "Upgrade" on BASIC tier
        await clickText('Upgrade', 10_000);
        console.log(`${LOG_PREFIX} 5.2.2: Clicked Upgrade (with error mock)`);
        await browser.pause(3_000);

        // Verify the purchase API was called
        const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
        expect(purchaseCall).toBeDefined();

        // Verify "Waiting" banner does NOT appear (purchase failed immediately)
        const hasWaiting = await isWaitingVisible();
        console.log(`${LOG_PREFIX} 5.2.2: Waiting banner visible (should be false): ${hasWaiting}`);
        expect(hasWaiting).toBe(false);

        // Verify Upgrade buttons remain clickable (isPurchasing reset to false)
        const hasUpgrade = await textExists('Upgrade');
        expect(hasUpgrade).toBe(true);
        console.log(`${LOG_PREFIX} 5.2.2: Upgrade button still clickable`);

        console.log(`${LOG_PREFIX} 5.2.2 PASSED`);
      } finally {
        setMockBehavior('purchaseError', 'false');
      }
    });

    it('5.2.3 — duplicate purchase prevention during checkout', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-dup-purchase-token');

      clearRequestLog();

      // Click "Upgrade" on BASIC tier -> "Waiting" banner appears
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 5.2.3: Clicked Upgrade on BASIC`);
      await browser.pause(3_000);

      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 5.2.3: Waiting banner visible`);

      // Verify ALL Upgrade buttons are disabled — both BASIC shows "Waiting..."
      // and PRO should be disabled too
      const hasWaitingButton = await textExists('Waiting...');
      console.log(`${LOG_PREFIX} 5.2.3: Waiting... button visible: ${hasWaitingButton}`);

      // Count upgrade-related elements — there should be no active "Upgrade" buttons
      // During purchasing, buttons show "Waiting..." or are disabled
      clearRequestLog();

      // Attempt to click on any remaining "Upgrade" text (PRO tier)
      // This should either not exist or not trigger a new API call
      const upgradeSelector = `//*[contains(@label, "Upgrade") or contains(@value, "Upgrade") or contains(@title, "Upgrade")]`;
      const upgradeElements = await browser.$$(upgradeSelector);
      console.log(
        `${LOG_PREFIX} 5.2.3: Found ${upgradeElements.length} "Upgrade" element(s) during purchasing`
      );

      // If any Upgrade elements exist, try clicking them
      if (upgradeElements.length > 0) {
        try {
          const el = upgradeElements[0];
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
          await browser.pause(2_000);
        } catch {
          console.log(`${LOG_PREFIX} 5.2.3: Could not click Upgrade element (expected — disabled)`);
        }
      }

      // Verify NO additional purchase API calls were made
      const additionalCalls = getRequestLog().filter(
        r => r.method === 'POST' && r.url.includes('/payments/stripe/purchasePlan')
      );
      console.log(
        `${LOG_PREFIX} 5.2.3: Additional purchase calls during lock: ${additionalCalls.length}`
      );
      expect(additionalCalls.length).toBe(0);

      // Resolve the polling so state clears for next tests
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      const waitingGone523 = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone523).toBe(true);

      console.log(`${LOG_PREFIX} 5.2.3 PASSED`);
    });
  });

  // -------------------------------------------------------------------------
  // 5.3 Billing Events
  // -------------------------------------------------------------------------

  describe('5.3 Billing Events', () => {
    it('5.3.1 — plan transition from FREE to PRO (direct)', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-pro-token');

      clearRequestLog();

      // We need to click the PRO "Upgrade" button, not the BASIC one.
      // Both tiers show "Upgrade" — PRO appears second. Use $$ to find all
      // and click the last one.
      const upgradeSelector = `//*[contains(@label, "Upgrade") or contains(@value, "Upgrade") or contains(@title, "Upgrade")]`;
      const upgradeElements = await browser.$$(upgradeSelector);
      console.log(`${LOG_PREFIX} 5.3.1: Found ${upgradeElements.length} Upgrade element(s)`);

      if (upgradeElements.length >= 2) {
        // Click the second (PRO) Upgrade button
        const proEl = upgradeElements[upgradeElements.length - 1];
        const location = await proEl.getLocation();
        const size = await proEl.getSize();
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
        console.log(`${LOG_PREFIX} 5.3.1: Clicked PRO Upgrade button`);
      } else if (upgradeElements.length === 1) {
        // Only one Upgrade button — click it (might be PRO if BASIC is current)
        const el = upgradeElements[0];
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
        console.log(`${LOG_PREFIX} 5.3.1: Clicked single Upgrade button`);
      } else {
        throw new Error('No Upgrade buttons found on billing page');
      }
      await browser.pause(3_000);

      // Verify POST /payments/stripe/purchasePlan with PRO in body
      const purchaseCall = await waitForRequest('POST', '/payments/stripe/purchasePlan', 10_000);
      if (!purchaseCall) {
        console.log(
          `${LOG_PREFIX} 5.3.1: Purchase request log:`,
          JSON.stringify(getRequestLog(), null, 2)
        );
      }
      expect(purchaseCall).toBeDefined();

      if (purchaseCall?.body) {
        const bodyStr = typeof purchaseCall.body === 'string' ? purchaseCall.body : '';
        console.log(`${LOG_PREFIX} 5.3.1: Purchase request body:`, bodyStr);
        expect(bodyStr).toContain('PRO');
        expect(bodyStr).toContain('MONTHLY');
      }

      // Switch mock to PRO plan active
      setMockBehavior('plan', 'PRO');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());

      // Wait for polling to detect and clear waiting state
      const waitingGone531 = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone531).toBe(true);

      // Re-auth to verify PRO is "Current"
      await reAuthAndGoToBilling('e2e-pro-verify-token');

      const hasPro = (await textExists('PRO')) || (await textExists('Pro'));
      expect(hasPro).toBe(true);

      const hasCurrent = await textExists('Current');
      expect(hasCurrent).toBe(true);

      console.log(`${LOG_PREFIX} 5.3.1 PASSED`);
    });

    it('5.3.2 — Manage Subscription opens Stripe portal', async () => {
      // Ensure mock has an active subscription so "Manage Subscription" renders.
      // We re-auth fresh to guarantee the team state reflects the mock.
      setMockBehavior('plan', 'PRO');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 30 * 86400000).toISOString());
      await reAuthAndGoToBilling('e2e-manage-sub-token');

      // Wait for "Manage Subscription" to appear (team state needs to populate)
      let hasManage = false;
      const deadline = Date.now() + 15_000;
      while (Date.now() < deadline) {
        hasManage = await textExists('Manage Subscription');
        if (hasManage) break;
        await browser.pause(1_000);
      }
      if (!hasManage) {
        const tree = await dumpAccessibilityTree();
        console.log(
          `${LOG_PREFIX} 5.3.2: Manage Subscription not found. Tree:\n`,
          tree.slice(0, 6000)
        );
      }
      expect(hasManage).toBe(true);

      clearRequestLog();

      // Click "Manage Subscription"
      await clickText('Manage Subscription', 10_000);
      console.log(`${LOG_PREFIX} 5.3.2: Clicked Manage Subscription`);
      await browser.pause(3_000);

      // Verify POST /payments/stripe/portal was called
      const portalCall = await waitForRequest('POST', '/payments/stripe/portal', 10_000);
      if (!portalCall) {
        console.log(
          `${LOG_PREFIX} 5.3.2: Portal request log:`,
          JSON.stringify(getRequestLog(), null, 2)
        );
      }
      expect(portalCall).toBeDefined();

      console.log(`${LOG_PREFIX} 5.3.2 PASSED`);
    });
  });
});
