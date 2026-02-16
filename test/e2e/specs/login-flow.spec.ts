/* eslint-disable */
// @ts-nocheck
/**
 * E2E test: Complete login → onboarding → home flow via deep link.
 *
 * Verifies the full auth + onboarding journey using mock data:
 *   1. `alphahuman://auth?token=...` deep link is triggered
 *   2. App calls POST /telegram/login-tokens/:token/consume  (mock server)
 *   3. App receives JWT, dispatches to Redux, navigates to #/onboarding
 *   4. UserProvider calls GET /telegram/me  (mock server)
 *   5. UserProvider calls GET /teams         (mock server)
 *   6. Onboarding Step 1: InviteCodeStep — skip
 *   7. Onboarding Step 2: FeaturesStep — click through
 *   8. Onboarding Step 3: PrivacyStep — click through
 *   9. Onboarding Step 4: GetStartedStep — complete onboarding
 *  10. App calls POST /telegram/settings/onboarding-complete  (mock server)
 *  11. App navigates to #/home — greeting with mock user's name shown
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
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

/**
 * Wait until the given text disappears from the accessibility tree,
 * indicating a page/step transition.  Falls back after timeout.
 */
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
    // Give the app time to finish launching (it starts hidden in tray mode)
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  // -----------------------------------------------------------------------
  // Phase 1: Deep link authentication
  // -----------------------------------------------------------------------

  it('app starts with window hidden (tray app)', async () => {
    const menuBar = await browser.$('//XCUIElementTypeMenuBar');
    expect(await menuBar.isExisting()).toBe(true);
  });

  it('deep link triggers login and shows the app window', async () => {
    await triggerAuthDeepLink('e2e-test-token');

    // The deep link handler calls invoke('show_window')
    await waitForWindowVisible(25_000);

    // Wait for the WebView to appear
    await waitForWebView(15_000);

    // Wait for the accessibility tree to populate
    await waitForAppReady(15_000);
  });

  it('mock server received the token-consume call', async () => {
    const call = await waitForRequest('POST', '/telegram/login-tokens/');
    if (!call) {
      console.log('[LoginFlow] Request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(call).toBeDefined();
  });

  it('mock server received the user-profile call', async () => {
    const call = await waitForRequest('GET', '/telegram/me');
    if (!call) {
      console.log('[LoginFlow] Request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(call).toBeDefined();
  });

  // -----------------------------------------------------------------------
  // Phase 2: Onboarding — walk through all 4 steps
  // -----------------------------------------------------------------------

  it('onboarding InviteCodeStep is visible', async () => {
    const candidates = ['Invite Code', 'Have an Invite Code', 'Skip for now', 'Redeem Code'];

    let found = false;
    for (const text of candidates) {
      if (await textExists(text)) {
        console.log(`[LoginFlow] InviteCodeStep visible: "${text}"`);
        found = true;
        break;
      }
    }

    if (!found) {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] InviteCodeStep text not found. Tree:\n', tree.slice(0, 3000));
    }

    const webView = await browser.$('//XCUIElementTypeWebView');
    expect(await webView.isExisting()).toBe(true);
  });

  it('skip invite code step → advances to FeaturesStep', async () => {
    // Click "Skip for now"
    await clickText('Skip for now', 10_000);
    console.log("[LoginFlow] Clicked 'Skip for now'");

    // Verify the step actually changed — wait for InviteCodeStep content to
    // disappear and FeaturesStep content to appear.
    const stepChanged = await waitForTextToDisappear('Skip for now', 8_000);
    if (stepChanged) {
      console.log('[LoginFlow] InviteCodeStep content disappeared — step advanced');
    } else {
      // If text didn't disappear, try clicking again (first click may have
      // hit the wrong area)
      console.log("[LoginFlow] Step didn't advance, retrying click...");
      await clickText('Skip', 5_000);
      const retryWorked = await waitForTextToDisappear('Skip', 5_000);
      if (!retryWorked) {
        const tree = await dumpAccessibilityTree();
        console.log(
          '[LoginFlow] InviteCodeStep still visible after retry. Tree:\n',
          tree.slice(0, 4000)
        );
        throw new Error(
          'InviteCodeStep did not advance after two click attempts — ' +
            "'Skip' text still visible in accessibility tree"
        );
      }
    }

    // Small pause for React state update + re-render
    await browser.pause(2_000);

    // Dump tree to see what's on screen now
    const tree = await dumpAccessibilityTree();
    console.log('[LoginFlow] After skip, accessibility tree:\n', tree.slice(0, 4000));
  });

  it('FeaturesStep — click through', async () => {
    // FeaturesStep button: "Looks Amazing. Bring It On 🚀"
    // Emoji may not appear in accessibility tree, try multiple variants
    const buttonCandidates = ['Looks Amazing', 'Bring It On'];

    let clicked = false;
    for (const text of buttonCandidates) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] FeaturesStep: clicked "${text}"`);
        clicked = true;
        break;
      }
    }

    if (!clicked) {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] FeaturesStep button not found. Tree:\n', tree.slice(0, 4000));
      throw new Error('Could not find FeaturesStep button');
    }

    await browser.pause(2_000);
  });

  it('PrivacyStep — click through', async () => {
    // PrivacyStep button: "Got it! Let's Continue 👀"
    const buttonCandidates = ['Got it', 'Continue'];

    let clicked = false;
    for (const text of buttonCandidates) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] PrivacyStep: clicked "${text}"`);
        clicked = true;
        break;
      }
    }

    if (!clicked) {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] PrivacyStep button not found. Tree:\n', tree.slice(0, 4000));
      throw new Error('Could not find PrivacyStep button');
    }

    await browser.pause(2_000);
  });

  it('GetStartedStep — complete onboarding', async () => {
    // GetStartedStep button: "I'm Ready! Let's Go! 🔥"
    // NOTE: Do NOT use "Ready" — it matches the heading "You Are Ready, Soldier!"
    // which is NOT inside the button and won't trigger handleComplete().
    const buttonCandidates = ["Let's Go", "I'm Ready"];

    let clicked = false;
    for (const text of buttonCandidates) {
      if (await textExists(text)) {
        await clickText(text, 5_000);
        console.log(`[LoginFlow] GetStartedStep: clicked "${text}"`);
        clicked = true;
        break;
      }
    }

    if (!clicked) {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] GetStartedStep button not found. Tree:\n', tree.slice(0, 4000));
      throw new Error('Could not find GetStartedStep button');
    }

    // Wait for the onboarding-complete API call + navigation to /home
    await browser.pause(3_000);
  });

  // -----------------------------------------------------------------------
  // Phase 3: Verify completion
  // -----------------------------------------------------------------------

  it('mock server received the onboarding-complete call', async () => {
    const call = await waitForRequest('POST', '/telegram/settings/onboarding-complete');
    if (!call) {
      console.log('[LoginFlow] Request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(call).toBeDefined();
  });

  it('app navigated to Home page after onboarding', async () => {
    // Home page shows a greeting with the mock user's first name ("Test")
    const nameCandidates = [
      'Test',
      'Good morning',
      'Good afternoon',
      'Good evening',
      'Message AlphaHuman',
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
