// @ts-nocheck
/**
 * E2E regression: onboarding overlay after logout -> re-login.
 *
 * Verifies:
 *   1. Initial login can complete onboarding and reach Home.
 *   2. Logout returns to Welcome/logged-out state.
 *   3. Re-login triggers the auth consume call on the mock backend.
 *   4. After re-login the mock /auth/me call is made (profile fetch).
 *   5. Onboarding overlay appears again after a fresh login (clean session).
 *
 * Note: auth tokens live in the in-process Rust core (not localStorage),
 * so this spec asserts UI-visible state (Welcome screen, onboarding overlay,
 * mock request log) rather than localStorage contents.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  logoutViaSettings,
  performFullLogin,
  waitForLoggedOutState,
  waitForOnboardingOverlayVisible,
  waitForRequest,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

describe('Logout -> re-login onboarding overlay', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    // Reach Welcome screen first (this spec drives login itself).
    await resetApp('e2e-logout-relogin-reset', { skipAuth: true });
    clearRequestLog();
    resetMockBehavior();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  it('shows onboarding overlay with clean state after logout and re-login', async function () {
    this.timeout(180_000);
    const hasChrome = await hasAppChrome();
    expect(hasChrome).toBe(true);

    // Step 1: Login, walk onboarding, reach Home.
    clearRequestLog();
    resetMockBehavior();
    await performFullLogin('e2e-logout-relogin-first-token', '[LogoutReLogin]');

    // Step 2: Logout via Settings.
    await logoutViaSettings('[LogoutReLogin]');

    // Verify logged-out state is visible (Welcome or Sign in).
    const loggedOutMarker = await waitForLoggedOutState(10_000);
    if (!loggedOutMarker) {
      const tree = await dumpAccessibilityTree();
      console.log('[LogoutReLogin] Logged-out state not visible. Tree:\n', tree.slice(0, 4000));
    }
    expect(loggedOutMarker).toBeTruthy();

    // Step 3: Re-login with a delayed /auth/me response so we can observe
    // the interim state.
    setMockBehavior('telegramMeDelayMs', '4500');
    clearRequestLog();

    await triggerAuthDeepLink('e2e-logout-relogin-second-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);

    // The mock must have received the consume call.
    const consumeCall = await waitForRequest(
      getRequestLog,
      'POST',
      '/telegram/login-tokens/',
      20_000
    );
    if (!consumeCall) {
      console.log(
        '[LogoutReLogin] Missing consume call on re-login. Request log:',
        JSON.stringify(getRequestLog(), null, 2)
      );
    }
    expect(consumeCall).toBeDefined();

    // Step 4: Verify the re-login triggered a profile fetch.
    const meCall = await waitForRequest(getRequestLog, 'GET', '/auth/me', 15_000);
    if (!meCall) {
      console.log(
        '[LogoutReLogin] Missing /auth/me call. Request log:',
        JSON.stringify(getRequestLog(), null, 2)
      );
    }
    expect(meCall).toBeDefined();

    // Step 5: After a fresh login (delayed profile fetch), the onboarding
    // overlay must eventually appear. Rely on the explicit overlay wait.
    const overlayVisible = await waitForOnboardingOverlayVisible(9_500);
    if (!overlayVisible) {
      const tree = await dumpAccessibilityTree();
      console.log(
        '[LogoutReLogin] Overlay did not appear after timeout. Tree:\n',
        tree.slice(0, 4000)
      );
      console.log(
        '[LogoutReLogin] Request log after timeout:',
        JSON.stringify(getRequestLog(), null, 2)
      );
    }
    expect(overlayVisible).toBe(true);

    expect(await textExists('Welcome')).toBe(true);
    expect(await textExists('Skip')).toBe(true);
  });
});
