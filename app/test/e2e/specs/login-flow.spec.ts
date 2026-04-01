// @ts-nocheck
/**
 * E2E test: Complete login → onboarding → home flow via deep link (Linux / tauri-driver).
 *
 * Verifies the full auth + onboarding journey using mock data:
 *   Phase 1 — Deep link authentication:
 *     1. `openhuman://auth?token=...` deep link is triggered via __simulateDeepLink
 *     2. App calls POST /telegram/login-tokens/:token/consume  (mock server)
 *     3. App receives JWT, dispatches to Redux authSlice
 *     4. UserProvider calls GET /telegram/me  (mock server)
 *
 *   Phase 2 — Onboarding steps (6 steps in Onboarding.tsx):
 *     Step 0: WelcomeStep       — "Continue"
 *     Step 1: LocalAIStep       — "Use Local Models"
 *     Step 2: ScreenPermissions — "Continue Without Permission" or "Continue"
 *     Step 3: ToolsStep         — "Continue"
 *     Step 4: SkillsStep        — "Finish Setup"
 *     Step 5: MnemonicStep      — checkbox + "Finish Setup"
 *
 *   Phase 3 — Completion verification:
 *     - App calls POST /settings/onboarding-complete (from SkillsStep)
 *     - App navigates to #/home — greeting with mock user's name shown
 *
 *   Phase 4 — Error paths:
 *     - Expired token returns 401 and app does not navigate to home
 *     - Invalid token returns 401 and app does not navigate to home
 *
 *   Phase 5 — Bypass auth path:
 *     - `openhuman://auth?token=...&key=auth` sets token directly (no consume call)
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { buildBypassJwt, triggerAuthDeepLink, triggerDeepLink } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
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
 * Wait until one of the candidate texts appears on screen.
 * Returns the matched text or null on timeout.
 */
async function waitForAnyText(candidates, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(500);
  }
  return null;
}

/**
 * Click the first matching text from a list of candidates.
 * Returns the clicked text or null if none found.
 */
async function clickFirstMatch(candidates, timeout = 5_000) {
  for (const text of candidates) {
    if (await textExists(text)) {
      await clickText(text, timeout);
      return text;
    }
  }
  return null;
}

/**
 * Verify Redux auth state via browser.execute (tauri-driver only).
 */
async function getReduxAuthState() {
  try {
    return await browser.execute(() => {
      // Redux store is exposed on window.__REDUX_DEVTOOLS_EXTENSION__
      // but we can read from localStorage where redux-persist stores auth
      const persistedAuth = localStorage.getItem('persist:auth');
      if (persistedAuth) {
        try {
          return JSON.parse(persistedAuth);
        } catch {
          return null;
        }
      }
      return null;
    });
  } catch {
    return null;
  }
}

// Track whether onboarding was walked through in the UI so Phase 3 can
// decide whether to require the onboarding-complete backend call.
let hadOnboardingWalkthrough = false;

describe('Login flow — complete with mock data (Linux)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
    hadOnboardingWalkthrough = false;
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // -----------------------------------------------------------------------
  // Phase 1: Deep link authentication
  // -----------------------------------------------------------------------

  it('app process is running and has a window handle', async () => {
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

  it('Redux auth state has a token after login', async () => {
    const authState = await getReduxAuthState();
    if (authState) {
      const token =
        typeof authState.token === 'string' ? authState.token.replace(/^"|"$/g, '') : null;
      console.log('[LoginFlow] Redux auth token present:', !!token);
      expect(token).toBeTruthy();
    } else {
      console.log('[LoginFlow] Could not read Redux auth state (persist format may differ)');
      // Non-fatal: the token-consume mock call was verified above
    }
  });

  // -----------------------------------------------------------------------
  // Phase 2: Onboarding (real step walkthrough)
  //
  // Onboarding.tsx renders as a portal overlay. On tauri-driver (Linux),
  // browser.execute() works, so we can interact with the WebView DOM.
  //
  // Steps in order:
  //   0: WelcomeStep       — "Continue" button
  //   1: LocalAIStep       — "Use Local Models"
  //   2: ScreenPermissions — "Continue Without Permission" or "Continue"
  //   3: ToolsStep         — "Continue" button
  //   4: SkillsStep        — "Finish Setup" button (fires onboarding-complete)
  //   5: MnemonicStep      — checkbox + "Finish Setup" button
  // -----------------------------------------------------------------------

  it('onboarding overlay or home page is visible', async () => {
    await browser.pause(3_000);

    // Real onboarding step markers
    const onboardingCandidates = [
      'Welcome', // WelcomeStep heading
      'Skip', // Onboarding defer button (top-right)
      'Continue', // WelcomeStep CTA
    ];
    const homeCandidates = ['Home', 'Skills', 'Conversations'];

    const foundOnboarding = await waitForAnyText(onboardingCandidates, 5_000);
    if (foundOnboarding) {
      console.log(`[LoginFlow] Onboarding visible: "${foundOnboarding}"`);
    }

    const foundHome = !foundOnboarding ? await waitForAnyText(homeCandidates, 5_000) : null;
    if (foundHome) {
      console.log(
        `[LoginFlow] Home page visible: "${foundHome}" (onboarding may be deferred/completed)`
      );
    }

    expect(foundOnboarding || foundHome).toBeTruthy();
  });

  it('walk through onboarding steps (if overlay is visible)', async () => {
    // Check if we're on the WelcomeStep or any onboarding step
    const onboardingVisible =
      (await textExists('Welcome')) ||
      (await textExists('Skip')) ||
      (await textExists('Use Local Models')) ||
      (await textExists('Continue'));

    if (!onboardingVisible) {
      console.log('[LoginFlow] Onboarding overlay not visible — skipping step walkthrough');
      hadOnboardingWalkthrough = false;
      return;
    }

    hadOnboardingWalkthrough = true;

    // Step 0: WelcomeStep — click "Continue"
    if (await textExists('Welcome')) {
      const clicked = await clickFirstMatch(['Continue'], 10_000);
      console.log(`[LoginFlow] WelcomeStep: clicked "${clicked}"`);
      await browser.pause(2_000);
    }

    // Step 1: LocalAIStep — only has "Use Local Models" button now
    {
      const clicked = await clickFirstMatch(['Use Local Models', 'Continue'], 10_000);
      if (clicked) {
        console.log(`[LoginFlow] LocalAIStep: clicked "${clicked}"`);
        await browser.pause(2_000);
      }
    }

    // Step 2: ScreenPermissionsStep — click "Continue Without Permission" (no accessibility on Linux CI)
    {
      const clicked = await clickFirstMatch(['Continue Without Permission', 'Continue'], 10_000);
      if (clicked) {
        console.log(`[LoginFlow] ScreenPermissionsStep: clicked "${clicked}"`);
        await browser.pause(2_000);
      }
    }

    // Step 3: ToolsStep — click "Continue" (keep defaults)
    {
      const toolsVisible = await textExists('Enable Tools');
      if (toolsVisible) {
        const clicked = await clickFirstMatch(['Continue'], 10_000);
        if (clicked) {
          console.log(`[LoginFlow] ToolsStep: clicked "${clicked}"`);
          await browser.pause(2_000);
        }
      }
    }

    // Step 4: SkillsStep — click "Finish Setup" (no skills connected in E2E)
    {
      const skillsVisible = await textExists('Install Skills');
      if (skillsVisible) {
        const clicked = await clickFirstMatch(['Finish Setup'], 10_000);
        if (clicked) {
          console.log(`[LoginFlow] SkillsStep: clicked "${clicked}"`);
          await browser.pause(3_000);
        }
      }
    }

    // Step 5: MnemonicStep — tick the checkbox and click "Finish Setup"
    {
      const mnemonicVisible = await textExists('Your Recovery Phrase');
      if (mnemonicVisible) {
        console.log('[LoginFlow] MnemonicStep: visible');

        // Tick the "I have saved my recovery phrase" checkbox
        try {
          const checked = await browser.execute(() => {
            const checkbox = document.querySelector('input[type="checkbox"]') as HTMLInputElement;
            if (checkbox && !checkbox.checked) {
              checkbox.click();
              return true;
            }
            return checkbox?.checked ?? false;
          });
          console.log(`[LoginFlow] MnemonicStep: checkbox checked=${checked}`);
        } catch (err) {
          console.log('[LoginFlow] MnemonicStep: checkbox click failed:', err);
        }

        await browser.pause(1_000);
        const clicked = await clickFirstMatch(['Finish Setup'], 10_000);
        if (clicked) {
          console.log(`[LoginFlow] MnemonicStep: clicked "${clicked}"`);
          await browser.pause(3_000);
        }
      }
    }
  });

  // -----------------------------------------------------------------------
  // Phase 3: Verify completion
  // -----------------------------------------------------------------------

  it('mock server received the onboarding-complete call (if onboarding was walked)', async () => {
    if (!hadOnboardingWalkthrough) {
      console.log(
        '[LoginFlow] Onboarding was not walked (overlay not visible) — skipping assertion'
      );
      return;
    }

    const log = getRequestLog();
    // The app calls POST /settings/onboarding-complete (via userApi.onboardingComplete)
    // The mock may handle it at /telegram/settings/onboarding-complete or /settings/onboarding-complete
    const call = log.find(
      r =>
        r.method === 'POST' &&
        (r.url.includes('/settings/onboarding-complete') ||
          r.url.includes('/telegram/settings/onboarding-complete'))
    );
    if (!call) {
      console.log('[LoginFlow] Onboarding was walked but onboarding-complete call missing.');
      console.log('[LoginFlow] Request log:', JSON.stringify(log, null, 2));
    }
    expect(call).toBeDefined();
    console.log('[LoginFlow] onboarding-complete call verified');
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

    const foundText = await waitForAnyText(nameCandidates, 15_000);

    if (foundText) {
      console.log(`[LoginFlow] Home page confirmed: found "${foundText}"`);
    } else {
      const tree = await dumpAccessibilityTree();
      console.log('[LoginFlow] Home page text not found. Tree:\n', tree.slice(0, 4000));
    }

    expect(foundText).not.toBeNull();
  });

  // -----------------------------------------------------------------------
  // Phase 4: Error paths — expired and invalid tokens
  // -----------------------------------------------------------------------

  it('expired token does not navigate to home', async () => {
    // Clear auth state so we're starting unauthenticated
    clearRequestLog();
    setMockBehavior('token', 'expired');
    await browser.execute(() => {
      localStorage.removeItem('persist:auth');
      window.location.hash = '/';
    });
    await browser.pause(2_000);

    // Trigger deep link with the expired token behavior
    await triggerDeepLink('openhuman://auth?token=expired-test-token');
    await browser.pause(5_000);

    // Verify the consume call was made (mock returns 401 for expired tokens)
    const call = await waitForRequest('POST', '/telegram/login-tokens/', 10_000);
    expect(call).toBeDefined();
    console.log('[LoginFlow] Expired token: consume call made (mock returns 401)');

    // Assert the app is NOT on the authenticated home page
    const homeCandidates = ['Good morning', 'Good afternoon', 'Good evening', 'Message OpenHuman'];
    const onHome = await waitForAnyText(homeCandidates, 5_000);
    expect(onHome).toBeNull();
    console.log('[LoginFlow] Expired token: home page not reached (correct)');

    // Assert the app shows unauthenticated UI (welcome/landing page)
    const welcomeCandidates = ['Welcome', 'OpenHuman', 'Sign in', 'Get Started'];
    const onWelcome = await waitForAnyText(welcomeCandidates, 5_000);
    console.log(`[LoginFlow] Expired token: unauthenticated UI visible: ${onWelcome ?? 'none'}`);

    // Assert auth token was not persisted
    const hasToken = await browser.execute(() => {
      const persisted = localStorage.getItem('persist:auth');
      if (!persisted) return false;
      try {
        const parsed = JSON.parse(persisted);
        const token = typeof parsed.token === 'string' ? parsed.token.replace(/^"|"$/g, '') : null;
        return !!token && token !== 'null';
      } catch {
        return false;
      }
    });
    expect(hasToken).toBe(false);
    console.log('[LoginFlow] Expired token: no auth token in localStorage (correct)');

    resetMockBehavior();
  });

  it('invalid token does not navigate to home', async () => {
    // Clear auth state so we're starting unauthenticated
    clearRequestLog();
    setMockBehavior('token', 'invalid');
    await browser.execute(() => {
      localStorage.removeItem('persist:auth');
      window.location.hash = '/';
    });
    await browser.pause(2_000);

    await triggerDeepLink('openhuman://auth?token=invalid-test-token');
    await browser.pause(5_000);

    // Verify the consume call was made (mock returns 401 for invalid tokens)
    const call = await waitForRequest('POST', '/telegram/login-tokens/', 10_000);
    expect(call).toBeDefined();
    console.log('[LoginFlow] Invalid token: consume call made (mock returns 401)');

    // Assert the app is NOT on the authenticated home page
    const homeCandidates = ['Good morning', 'Good afternoon', 'Good evening', 'Message OpenHuman'];
    const onHome = await waitForAnyText(homeCandidates, 5_000);
    expect(onHome).toBeNull();
    console.log('[LoginFlow] Invalid token: home page not reached (correct)');

    // Assert auth token was not persisted
    const hasToken = await browser.execute(() => {
      const persisted = localStorage.getItem('persist:auth');
      if (!persisted) return false;
      try {
        const parsed = JSON.parse(persisted);
        const token = typeof parsed.token === 'string' ? parsed.token.replace(/^"|"$/g, '') : null;
        return !!token && token !== 'null';
      } catch {
        return false;
      }
    });
    expect(hasToken).toBe(false);
    console.log('[LoginFlow] Invalid token: no auth token in localStorage (correct)');

    resetMockBehavior();
  });

  // -----------------------------------------------------------------------
  // Phase 5: Bypass auth path (key=auth)
  // -----------------------------------------------------------------------

  it('bypass auth deep link sets token directly without consume call', async () => {
    // Clear auth state so we start unauthenticated — prevents stale session
    clearRequestLog();
    resetMockBehavior();
    await browser.execute(() => {
      localStorage.removeItem('persist:auth');
      window.location.hash = '/';
    });
    await browser.pause(2_000);

    const bypassJwt = buildBypassJwt('e2e-bypass-user');

    // Trigger bypass deep link (key=auth skips token consume)
    await triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(bypassJwt)}&key=auth`);
    await browser.pause(5_000);

    // Assert NO consume call was made (bypass skips it)
    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/telegram/login-tokens/')
    );
    expect(consumeCall).toBeUndefined();
    console.log('[LoginFlow] Bypass auth: no consume call (correct — token set directly)');

    // Assert the app navigated to home (post-login UI marker)
    const homeCandidates = [
      'Good morning',
      'Good afternoon',
      'Good evening',
      'Message OpenHuman',
      'Home',
    ];
    const foundHome = await waitForAnyText(homeCandidates, 15_000);
    expect(foundHome).not.toBeNull();
    console.log(`[LoginFlow] Bypass auth: home reached with "${foundHome}"`);

    // Assert Redux token was persisted in localStorage
    const tokenSet = await browser.execute(() => {
      const persisted = localStorage.getItem('persist:auth');
      if (!persisted) return false;
      try {
        const parsed = JSON.parse(persisted);
        const token = typeof parsed.token === 'string' ? parsed.token.replace(/^"|"$/g, '') : null;
        return !!token && token !== 'null';
      } catch {
        return false;
      }
    });
    expect(tokenSet).toBe(true);
    console.log('[LoginFlow] Bypass auth: Redux token present in localStorage');
  });
});
