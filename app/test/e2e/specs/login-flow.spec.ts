// @ts-nocheck
/**
 * E2E test: Complete login → onboarding → home flow via deep link.
 *
 * Verifies the full auth + onboarding journey using mock data:
 *   1. `openhuman://auth?token=...` deep link is triggered
 *   2. App calls POST /telegram/login-tokens/:token/consume  (mock server)
 *   3. App receives JWT, dispatches to Redux, navigates to #/home
 *   4. UserProvider calls GET /telegram/me  (mock server)
 *   5. Onboarding overlay may appear (React portal — not always visible on Mac2)
 *   6. App navigates to #/home — greeting with mock user's name shown
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

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

async function waitForTextToDisappear(text, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await textExists(text))) return true;
    await browser.pause(500);
  }
  return false;
}

describe('Login flow — complete with mock data', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  // -----------------------------------------------------------------------
  // Phase 1: Deep link authentication
  // -----------------------------------------------------------------------

  it('app process is running and has chrome (menu bar on macOS, window on Linux)', async () => {
    const hasChrome = await hasAppChrome();
    expect(hasChrome).toBe(true);
  });

  it('deep link triggers login and shows the app window', async () => {
    await triggerAuthDeepLink('e2e-test-token');

    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);
  });

  it('mock server received the token-consume call', async () => {
    const call = await waitForRequest('POST', '/telegram/login-tokens/', 20_000);
    if (!call) {
      console.log(
        '[LoginFlow] Missing consume call. Request log:',
        JSON.stringify(getRequestLog(), null, 2)
      );
    }
    expect(call).toBeDefined();
  });

  it('mock server received the user-profile call', async () => {
    const deadline = Date.now() + 15_000;
    let call;
    while (Date.now() < deadline) {
      const log = getRequestLog();
      call = log.find(
        r => r.method === 'GET' && (r.url.includes('/telegram/me') || r.url.includes('/settings'))
      );
      if (call) break;
      await browser.pause(500);
    }
    if (!call) {
      console.log('[LoginFlow] Request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(call).toBeDefined();
  });

  // -----------------------------------------------------------------------
  // Phase 2: Onboarding (conditional — portal may not be visible on Mac2)
  // -----------------------------------------------------------------------

  it('onboarding overlay or home page is visible', async () => {
    await browser.pause(3_000);

    const onboardingCandidates = [
      'Invite Code',
      'Have an Invite Code',
      'Skip for now',
      'Redeem Code',
    ];
    const homeCandidates = ['Home', 'Skills', 'Conversations'];

    let foundOnboarding = false;
    let foundHome = false;

    for (const text of onboardingCandidates) {
      if (await textExists(text)) {
        console.log(`[LoginFlow] Onboarding visible: "${text}"`);
        foundOnboarding = true;
        break;
      }
    }

    if (!foundOnboarding) {
      for (const text of homeCandidates) {
        if (await textExists(text)) {
          console.log(
            `[LoginFlow] Home page visible: "${text}" (onboarding overlay may be hidden from accessibility tree)`
          );
          foundHome = true;
          break;
        }
      }
    }

    expect(foundOnboarding || foundHome).toBe(true);
  });

  it('walk through onboarding steps (if overlay is visible)', async () => {
    const skipVisible = await textExists('Skip for now');

    if (!skipVisible) {
      console.log(
        '[LoginFlow] Onboarding overlay not visible in accessibility tree — skipping step walkthrough'
      );
      console.log(
        '[LoginFlow] (This is expected on Mac2 due to WKWebView portal accessibility limitations)'
      );
      return;
    }

    // Step 1: Skip invite code
    await clickText('Skip for now', 10_000);
    console.log("[LoginFlow] Clicked 'Skip for now'");
    await waitForTextToDisappear('Skip for now', 8_000);
    await browser.pause(2_000);

    // Step 2: FeaturesStep
    for (const text of ['Looks Amazing', 'Bring It On']) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] FeaturesStep: clicked "${text}"`);
        break;
      }
    }
    await browser.pause(2_000);

    // Step 3: PrivacyStep
    for (const text of ['Got it', 'Continue']) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] PrivacyStep: clicked "${text}"`);
        break;
      }
    }
    await browser.pause(2_000);

    // Step 4: GetStartedStep
    for (const text of ["Let's Go", "I'm Ready"]) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] GetStartedStep: clicked "${text}"`);
        break;
      }
    }
    await browser.pause(3_000);
  });

  // -----------------------------------------------------------------------
  // Phase 3: Verify completion
  // -----------------------------------------------------------------------

  it('mock server received the onboarding-complete call (if onboarding was walked)', async () => {
    const log = getRequestLog();
    const call = log.find(
      r => r.method === 'POST' && r.url.includes('/telegram/settings/onboarding-complete')
    );
    if (!call) {
      const hadOnboarding = log.some(r => r.url.includes('onboarding'));
      if (!hadOnboarding) {
        console.log(
          '[LoginFlow] Onboarding was not walked (overlay not visible) — skipping assertion'
        );
        return;
      }
      console.log('[LoginFlow] Request log:', JSON.stringify(log, null, 2));
    }
    expect(call).toBeDefined();
  });

  it('app navigated to Home page after onboarding', async () => {
    const nameCandidates = [
      'Test',
      'Good morning',
      'Good afternoon',
      'Good evening',
      'Message OpenHuman',
      'Upgrade to Premium',
    ];

    let foundText = null;
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      for (const text of nameCandidates) {
        if (await textExists(text)) {
          foundText = text;
          break;
        }
      }
      if (foundText) break;
      await browser.pause(1_000);
    }

    if (foundText) {
      console.log(`[LoginFlow] Home page confirmed: found "${foundText}"`);
    } else {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] Home page text not found. Tree:\n', tree.slice(0, 4000));
    }

    expect(foundText).not.toBeNull();
  });
});
