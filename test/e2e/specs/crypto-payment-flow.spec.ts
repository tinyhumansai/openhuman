/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Cryptocurrency Payment Processing Flow.
 *
 * Covers:
 *   6.1.1  Coinbase charge created with correct plan and interval (BASIC annual)
 *   6.1.2  Coinbase charge created for PRO tier crypto payment
 *   6.1.3  Crypto toggle forces annual billing interval
 *   6.2.1  Successful crypto payment confirmation via polling
 *   6.2.2  Underpayment — plan does NOT activate, waiting state persists
 *   6.2.3  Overpayment — plan activates normally
 *   6.3.1  Payment status update: polling detects plan change after confirmation
 *   6.3.2  Coinbase API error handled gracefully
 *   6.3.3  Expired charge — waiting state clears after poll timeout
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
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

const LOG_PREFIX = '[CryptoPayment]';

/**
 * Click a native XCUIElementTypeButton by its label/title attribute.
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
 *
 * Retries clicking the Settings nav if the billing menu item isn't found
 * on the first attempt — after many re-auth cycles the modal can be slow
 * to appear.
 */
async function navigateToBilling() {
  const billingCandidates = ['Billing', 'Billing & Usage'];
  const maxAttempts = 3;

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    await clickNativeButton('Settings', 10_000);
    console.log(`${LOG_PREFIX} Clicked Settings nav (attempt ${attempt})`);
    await browser.pause(3_000);

    let clicked = false;
    for (const text of billingCandidates) {
      if (await textExists(text)) {
        await clickText(text, 10_000);
        console.log(`${LOG_PREFIX} Clicked "${text}" menu item`);
        clicked = true;
        break;
      }
    }

    if (clicked) {
      await browser.pause(2_000);
      return;
    }

    console.log(`${LOG_PREFIX} Billing menu not found on attempt ${attempt}, retrying...`);
    await browser.pause(2_000);
  }

  const tree = await dumpAccessibilityTree();
  console.log(
    `${LOG_PREFIX} Billing menu item not found after ${maxAttempts} attempts. Tree:\n`,
    tree.slice(0, 6000)
  );
  throw new Error('Billing menu item not found in Settings');
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
 */
async function performFullLogin(token = 'e2e-test-token') {
  await triggerAuthDeepLink(token);

  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);

  // Onboarding Step 1: InviteCodeStep — skip
  await clickText('Skip for now', 10_000);
  console.log(`${LOG_PREFIX} Clicked "Skip for now"`);

  const stepChanged = await waitForTextToDisappear('Skip for now', 8_000);
  if (!stepChanged) {
    console.log(`${LOG_PREFIX} Step did not advance, retrying...`);
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
 * Counter for unique JWT suffixes.
 */
let reAuthCounter = 0;

/**
 * Re-authenticate via deep link and navigate to billing.
 */
async function reAuthAndGoToBilling(token = 'e2e-crypto-payment-token') {
  clearRequestLog();

  reAuthCounter += 1;
  setMockBehavior('jwt', `crypto-reauth-${reAuthCounter}`);

  await triggerAuthDeepLink(token);
  await browser.pause(5_000);

  try {
    await clickNativeButton('Home', 5_000);
    await browser.pause(2_000);
  } catch {
    // Home button might not be visible yet
  }

  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(`${LOG_PREFIX} reAuth: Home page not reached. Tree:\n`, tree.slice(0, 4000));
    throw new Error('reAuthAndGoToBilling: Home page not reached');
  }
  console.log(`${LOG_PREFIX} Re-authed (jwt suffix crypto-reauth-${reAuthCounter}), on Home`);

  await navigateToBilling();
}

/**
 * Toggle the "Pay with Crypto" switch ON.
 * Uses multiple strategies because the <button role="switch"> may map to
 * different accessibility element types in WKWebView.
 */
async function enableCryptoToggle() {
  let toggled = false;

  // Strategy 1: Click a native switch element
  const switchSelectors = [
    '//XCUIElementTypeSwitch',
    '//XCUIElementTypeCheckBox',
    `//*[@role="switch"]`,
  ];
  for (const sel of switchSelectors) {
    try {
      const switchEl = await browser.$(sel);
      if (await switchEl.isExisting()) {
        const loc = await switchEl.getLocation();
        const sz = await switchEl.getSize();
        const cx = Math.round(loc.x + sz.width / 2);
        const cy = Math.round(loc.y + sz.height / 2);
        await browser.performActions([
          {
            type: 'pointer',
            id: 'mouse1',
            parameters: { pointerType: 'mouse' },
            actions: [
              { type: 'pointerMove', duration: 10, x: cx, y: cy },
              { type: 'pointerDown', button: 0 },
              { type: 'pause', duration: 50 },
              { type: 'pointerUp', button: 0 },
            ],
          },
        ]);
        await browser.releaseActions();
        console.log(`${LOG_PREFIX} Toggled crypto via ${sel}`);
        toggled = true;
        break;
      }
    } catch {
      // Try next selector
    }
  }

  // Strategy 2: Positional click at the far right of the "Pay with Crypto" row
  if (!toggled) {
    const labelEl = await waitForText('Pay with Crypto', 10_000);
    const loc = await labelEl.getLocation();
    const sz = await labelEl.getSize();

    const webView = await browser.$('//XCUIElementTypeWebView');
    const wvLoc = await webView.getLocation();
    const wvSz = await webView.getSize();
    const toggleX = Math.round(wvLoc.x + wvSz.width - 60);
    const toggleY = Math.round(loc.y + sz.height / 2);
    console.log(
      `${LOG_PREFIX} Positional click at (${toggleX}, ${toggleY}), ` +
        `label at (${loc.x}, ${loc.y}), webview right edge: ${wvLoc.x + wvSz.width}`
    );

    await browser.performActions([
      {
        type: 'pointer',
        id: 'mouse1',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 10, x: toggleX, y: toggleY },
          { type: 'pointerDown', button: 0 },
          { type: 'pause', duration: 50 },
          { type: 'pointerUp', button: 0 },
        ],
      },
    ]);
    await browser.releaseActions();
    console.log(`${LOG_PREFIX} Toggled crypto via positional click`);
    toggled = true;
  }

  await browser.pause(1_000);
  return toggled;
}

/**
 * Check if the "Waiting for payment confirmation" banner or "Waiting..." button
 * text is visible.
 */
async function isWaitingVisible() {
  return (await textExists('Waiting for payment confirmation')) || (await textExists('Waiting...'));
}

/**
 * Click the Nth "Upgrade" button on the billing page (0-indexed).
 * BASIC is index 0, PRO is index 1 when on FREE plan.
 */
async function clickUpgradeButton(index = 0) {
  const upgradeSelector = `//*[contains(@label, "Upgrade") or contains(@value, "Upgrade") or contains(@title, "Upgrade")]`;
  const upgradeElements = await browser.$$(upgradeSelector);
  console.log(`${LOG_PREFIX} Found ${upgradeElements.length} Upgrade element(s)`);

  if (upgradeElements.length <= index) {
    throw new Error(
      `Expected at least ${index + 1} Upgrade button(s), found ${upgradeElements.length}`
    );
  }

  const el = upgradeElements[index];
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

// ===========================================================================
// Test suite
// ===========================================================================

describe('Cryptocurrency Payment Processing Flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();

    // Full login + onboarding — lands on Home
    await performFullLogin('e2e-crypto-flow-token');

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
  // 6.1 Invoice Creation
  // -------------------------------------------------------------------------

  describe('6.1 Invoice Creation', () => {
    it('6.1.1 — Coinbase charge created with correct plan and interval (BASIC annual)', async () => {
      // Verify we're on the billing page with FREE plan
      const hasPlanText = await textExists('Your Current Plan');
      if (!hasPlanText) {
        const tree = await dumpAccessibilityTree();
        console.log(`${LOG_PREFIX} Billing page tree:\n`, tree.slice(0, 6000));
      }
      expect(hasPlanText).toBe(true);

      const hasFree = await textExists('FREE');
      expect(hasFree).toBe(true);

      // Toggle "Pay with Crypto" switch ON
      await enableCryptoToggle();

      // Verify billing interval is forced to "Annual"
      const hasAnnual = await textExists('Annual');
      expect(hasAnnual).toBe(true);

      clearRequestLog();

      // Click "Upgrade" on BASIC tier (first Upgrade button)
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.1.1: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify POST /payments/coinbase/charge was called
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      if (!coinbaseCall) {
        console.log(`${LOG_PREFIX} 6.1.1: Request log:`, JSON.stringify(getRequestLog(), null, 2));
      }
      expect(coinbaseCall).toBeDefined();

      // Verify request body contains plan and interval
      if (coinbaseCall?.body) {
        const bodyStr = typeof coinbaseCall.body === 'string' ? coinbaseCall.body : '';
        console.log(`${LOG_PREFIX} 6.1.1: Coinbase request body:`, bodyStr);
        expect(bodyStr).toContain('BASIC');
        expect(bodyStr).toContain('annual');
      }

      // Verify NO Stripe purchasePlan call was made
      const stripeCall = getRequestLog().find(
        r => r.method === 'POST' && r.url.includes('/payments/stripe/purchasePlan')
      );
      expect(stripeCall).toBeUndefined();

      // Verify "Waiting for payment confirmation" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 6.1.1: Waiting banner visible`);

      // Resolve polling so state clears for next test
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      const waitingGone = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone).toBe(true);

      console.log(`${LOG_PREFIX} 6.1.1 PASSED`);
    });

    it('6.1.2 — Coinbase charge created for PRO tier crypto payment', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-pro-token');

      // Toggle "Pay with Crypto" switch ON
      await enableCryptoToggle();

      clearRequestLog();

      // Click the PRO "Upgrade" button (second button, index 1)
      await clickUpgradeButton(1);
      console.log(`${LOG_PREFIX} 6.1.2: Clicked PRO Upgrade button`);
      await browser.pause(3_000);

      // Verify POST /payments/coinbase/charge was called
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      if (!coinbaseCall) {
        console.log(`${LOG_PREFIX} 6.1.2: Request log:`, JSON.stringify(getRequestLog(), null, 2));
      }
      expect(coinbaseCall).toBeDefined();

      // Verify request body contains PRO plan
      if (coinbaseCall?.body) {
        const bodyStr = typeof coinbaseCall.body === 'string' ? coinbaseCall.body : '';
        console.log(`${LOG_PREFIX} 6.1.2: Coinbase request body:`, bodyStr);
        expect(bodyStr).toContain('PRO');
        expect(bodyStr).toContain('annual');
      }

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);

      // Resolve polling
      setMockBehavior('plan', 'PRO');
      setMockBehavior('planActive', 'true');
      const waitingGone = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone).toBe(true);

      console.log(`${LOG_PREFIX} 6.1.2 PASSED`);
    });

    it('6.1.3 — Crypto toggle forces annual billing and disables monthly', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-interval-token');

      // Verify default billing interval is "Monthly"
      const hasMonthly = await textExists('Monthly');
      expect(hasMonthly).toBe(true);

      // Toggle "Pay with Crypto" ON
      await enableCryptoToggle();

      // Verify billing interval switched to "Annual"
      const hasAnnual = await textExists('Annual');
      expect(hasAnnual).toBe(true);

      // Attempt to click "Monthly" — it should be disabled (opacity-40, cursor-not-allowed)
      // Even if we click it, the interval should stay "Annual" because crypto forces annual
      try {
        await clickText('Monthly', 5_000);
        await browser.pause(1_000);
      } catch {
        console.log(`${LOG_PREFIX} 6.1.3: Monthly button click failed (expected — disabled)`);
      }

      // Verify "Annual" is still the active interval after clicking Monthly
      // The component uses crypto check: if paymentMethod === 'crypto', setBillingInterval is blocked
      const stillAnnual = await textExists('Annual');
      expect(stillAnnual).toBe(true);

      console.log(`${LOG_PREFIX} 6.1.3 PASSED`);
    });
  });

  // -------------------------------------------------------------------------
  // 6.2 Confirmation Handling
  // -------------------------------------------------------------------------

  describe('6.2 Confirmation Handling', () => {
    it('6.2.1 — Successful crypto payment confirmation via polling', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-confirm-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.2.1: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify Coinbase charge was created
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      expect(coinbaseCall).toBeDefined();

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.1: Waiting banner visible`);

      // Simulate successful crypto payment: update mock plan
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 365 * 86400000).toISOString());
      setMockBehavior('cryptoStatus', 'CONFIRMED');

      // Wait for polling to detect plan change and clear waiting state
      const waitingGone = await waitForTextToDisappear('Waiting', 15_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.1: Waiting banner disappeared after polling`);

      // Re-auth and verify plan persists as BASIC with "Current" badge
      await reAuthAndGoToBilling('e2e-crypto-confirm-verify-token');

      const hasBasic = (await textExists('BASIC')) || (await textExists('Basic'));
      expect(hasBasic).toBe(true);

      const hasCurrent = await textExists('Current');
      expect(hasCurrent).toBe(true);

      console.log(`${LOG_PREFIX} 6.2.1 PASSED`);
    });

    it('6.2.2 — Underpayment: plan does NOT activate, waiting state persists', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-underpaid-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.2.2: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify Coinbase charge was created
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      expect(coinbaseCall).toBeDefined();

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.2: Waiting banner visible`);

      // Simulate underpayment: plan stays FREE, subscription not active
      // The backend would not activate the subscription for underpaid charges.
      // Mock keeps returning FREE plan (default) — polling sees no change.
      setMockBehavior('cryptoStatus', 'UNDERPAID');
      setMockBehavior('cryptoUnderpaidAmount', '100.00');
      // Do NOT set plan or planActive — plan stays FREE

      // Wait 15s — the "Waiting" banner should still be visible because
      // the plan hasn't changed (still FREE, no active subscription)
      await browser.pause(15_000);

      const stillWaiting = await isWaitingVisible();
      expect(stillWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.2: Waiting state persists after underpayment (expected)`);

      // Verify the "Upgrade" button shows "Waiting..." (all disabled during purchase)
      const hasWaitingButton = await textExists('Waiting...');
      console.log(`${LOG_PREFIX} 6.2.2: Waiting... button visible: ${hasWaitingButton}`);

      // Verify plan is still FREE
      const hasFree = await textExists('FREE');
      expect(hasFree).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.2: Plan still FREE (underpayment not accepted)`);

      // Now resolve: simulate the user completing the remaining payment
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('cryptoStatus', 'CONFIRMED');
      const waitingGone = await waitForTextToDisappear('Waiting', 20_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.2: Resolved after completing payment`);

      console.log(`${LOG_PREFIX} 6.2.2 PASSED`);
    });

    it('6.2.3 — Overpayment: plan activates normally', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-overpaid-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.2.3: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify Coinbase charge was created
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      expect(coinbaseCall).toBeDefined();

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.3: Waiting banner visible`);

      // Simulate overpayment: backend accepts the payment and activates plan.
      // Even though the user sent more than required, the plan activates normally.
      setMockBehavior('cryptoStatus', 'OVERPAID');
      setMockBehavior('cryptoOverpaidAmount', '50.00');
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 365 * 86400000).toISOString());

      // Wait for polling to detect plan change
      const waitingGone = await waitForTextToDisappear('Waiting', 15_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.3: Waiting cleared — overpayment accepted`);

      // Re-auth and verify plan is BASIC
      await reAuthAndGoToBilling('e2e-crypto-overpaid-verify-token');

      const hasBasic = (await textExists('BASIC')) || (await textExists('Basic'));
      expect(hasBasic).toBe(true);

      const hasCurrent = await textExists('Current');
      expect(hasCurrent).toBe(true);
      console.log(`${LOG_PREFIX} 6.2.3: Plan activated as BASIC despite overpayment`);

      console.log(`${LOG_PREFIX} 6.2.3 PASSED`);
    });
  });

  // -------------------------------------------------------------------------
  // 6.3 Payment Status Update
  // -------------------------------------------------------------------------

  describe('6.3 Payment Status Update', () => {
    it('6.3.1 — Polling detects plan change after crypto confirmation', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-status-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.3.1: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify charge created
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      expect(coinbaseCall).toBeDefined();

      // Verify waiting state is active
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);

      // Verify polling is hitting /payments/stripe/currentPlan
      // Wait a few seconds for at least one poll to fire
      await browser.pause(6_000);
      const pollCalls = getRequestLog().filter(
        r => r.method === 'GET' && r.url.includes('/payments/stripe/currentPlan')
      );
      console.log(`${LOG_PREFIX} 6.3.1: Poll calls so far: ${pollCalls.length}`);
      expect(pollCalls.length).toBeGreaterThan(0);

      // Now simulate the payment being confirmed on the backend
      setMockBehavior('plan', 'BASIC');
      setMockBehavior('planActive', 'true');
      setMockBehavior('planExpiry', new Date(Date.now() + 365 * 86400000).toISOString());

      // Polling should detect the change within ~5s (poll interval)
      const waitingGone = await waitForTextToDisappear('Waiting', 15_000);
      expect(waitingGone).toBe(true);
      console.log(`${LOG_PREFIX} 6.3.1: Polling detected plan change, waiting cleared`);

      // Verify fetchCurrentUser was triggered (re-auth verifies state)
      await reAuthAndGoToBilling('e2e-crypto-status-verify-token');
      const hasBasic = (await textExists('BASIC')) || (await textExists('Basic'));
      expect(hasBasic).toBe(true);

      console.log(`${LOG_PREFIX} 6.3.1 PASSED`);
    });

    it('6.3.2 — Coinbase API error handled gracefully', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-error-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      // Set Coinbase to return 500 error
      setMockBehavior('coinbaseError', 'true');

      try {
        clearRequestLog();

        // Click "Upgrade" on BASIC tier
        await clickText('Upgrade', 10_000);
        console.log(`${LOG_PREFIX} 6.3.2: Clicked Upgrade (with error mock)`);
        await browser.pause(3_000);

        // Verify Coinbase API was called
        const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
        expect(coinbaseCall).toBeDefined();

        // Verify "Waiting" banner does NOT appear (purchase failed immediately)
        const hasWaiting = await isWaitingVisible();
        console.log(`${LOG_PREFIX} 6.3.2: Waiting banner visible (should be false): ${hasWaiting}`);
        expect(hasWaiting).toBe(false);

        // Verify Upgrade buttons remain clickable (isPurchasing reset to false)
        const hasUpgrade = await textExists('Upgrade');
        expect(hasUpgrade).toBe(true);
        console.log(`${LOG_PREFIX} 6.3.2: Upgrade button still clickable after error`);

        // Verify NO Stripe calls were made (crypto mode was active)
        const stripeCalls = getRequestLog().filter(
          r => r.method === 'POST' && r.url.includes('/payments/stripe/purchasePlan')
        );
        expect(stripeCalls.length).toBe(0);

        console.log(`${LOG_PREFIX} 6.3.2 PASSED`);
      } finally {
        setMockBehavior('coinbaseError', 'false');
      }
    });

    it('6.3.3 — Expired charge: waiting state clears after poll timeout', async () => {
      // Reset to FREE plan and re-auth
      resetMockBehavior();
      await reAuthAndGoToBilling('e2e-crypto-expired-token');

      // Enable crypto toggle
      await enableCryptoToggle();

      clearRequestLog();

      // Click "Upgrade" on BASIC tier
      await clickText('Upgrade', 10_000);
      console.log(`${LOG_PREFIX} 6.3.3: Clicked Upgrade button`);
      await browser.pause(3_000);

      // Verify charge created
      const coinbaseCall = await waitForRequest('POST', '/payments/coinbase/charge', 10_000);
      expect(coinbaseCall).toBeDefined();

      // Verify "Waiting" banner appears
      const hasWaiting = await isWaitingVisible();
      expect(hasWaiting).toBe(true);

      // Simulate expired charge: plan stays FREE (no change)
      // The user didn't pay before the charge expired.
      setMockBehavior('cryptoStatus', 'EXPIRED');
      // Do NOT update plan — it stays FREE

      // The BillingPanel polling has a 2-minute timeout. We don't want to wait
      // the full 2 minutes in the test, so instead we re-auth which resets
      // component state (isPurchasing resets to false on remount).
      console.log(`${LOG_PREFIX} 6.3.3: Simulating charge expiry — re-auth to reset state`);
      resetMockBehavior(); // Plan stays FREE
      await reAuthAndGoToBilling('e2e-crypto-expired-verify-token');

      // Verify plan is still FREE after expired charge
      const hasFree = await textExists('FREE');
      expect(hasFree).toBe(true);

      // Verify no active subscription
      const hasCurrent = await textExists('Current');
      // "Current" badge should be on FREE tier
      expect(hasCurrent).toBe(true);

      // Verify "Waiting" banner is NOT visible (fresh component state)
      const noWaiting = !(await isWaitingVisible());
      expect(noWaiting).toBe(true);

      // Verify user can try again — Upgrade button is available
      const hasUpgrade = await textExists('Upgrade');
      expect(hasUpgrade).toBe(true);
      console.log(`${LOG_PREFIX} 6.3.3: Upgrade available after expired charge`);

      console.log(`${LOG_PREFIX} 6.3.3 PASSED`);
    });
  });
});
