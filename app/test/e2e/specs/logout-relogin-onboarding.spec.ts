// @ts-nocheck
/**
 * E2E regression: onboarding overlay after logout -> re-login.
 *
 * Verifies:
 *   1. Initial login can complete onboarding and reach Home.
 *   2. Logout clears persisted auth/onboarding state.
 *   3. Re-login with a delayed profile fetch does not show onboarding immediately
 *      (proves no stale local timeout state leaked across sessions).
 *   4. Once the fresh-session timeout path elapses, onboarding overlay appears
 *      again with the expected clean-state entry markers.
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
import {
  isOnboardingOverlayVisible,
  logoutViaSettings,
  performFullLogin,
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

function parsePersistedValue(value) {
  if (typeof value !== 'string') return value;

  try {
    return JSON.parse(value);
  } catch {
    return value.replace(/^"|"$/g, '');
  }
}

async function getPersistedAuthSnapshot() {
  return browser.execute(() => {
    const raw = window.localStorage.getItem('persist:auth');
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw);
      const decode = value => {
        if (typeof value !== 'string') return value;
        try {
          return JSON.parse(value);
        } catch {
          return value.replace(/^"|"$/g, '');
        }
      };

      return {
        token: decode(parsed.token),
        isOnboardedByUser: decode(parsed.isOnboardedByUser),
        onboardingTasksByUser: decode(parsed.onboardingTasksByUser),
        hasIncompleteOnboardingByUser: decode(parsed.hasIncompleteOnboardingByUser),
        isAnalyticsEnabledByUser: decode(parsed.isAnalyticsEnabledByUser),
      };
    } catch {
      return null;
    }
  });
}

async function waitForPersistedAuthReset(timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const snapshot = await getPersistedAuthSnapshot();
    if (
      snapshot &&
      !snapshot.token &&
      Object.keys(snapshot.isOnboardedByUser || {}).length === 0 &&
      Object.keys(snapshot.onboardingTasksByUser || {}).length === 0 &&
      Object.keys(snapshot.hasIncompleteOnboardingByUser || {}).length === 0 &&
      Object.keys(snapshot.isAnalyticsEnabledByUser || {}).length === 0
    ) {
      return snapshot;
    }
    await browser.pause(400);
  }
  return null;
}

async function waitForPersistedAuthToken(timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const snapshot = await getPersistedAuthSnapshot();
    if (snapshot?.token) {
      return snapshot;
    }
    await browser.pause(400);
  }
  return null;
}

describe('Logout -> re-login onboarding overlay', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
    resetMockBehavior();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  it('shows onboarding overlay with clean state after logout and re-login', async () => {
    const hasChrome = await hasAppChrome();
    expect(hasChrome).toBe(true);

    clearRequestLog();
    resetMockBehavior();
    await performFullLogin('e2e-logout-relogin-first-token', '[LogoutReLogin]');

    await logoutViaSettings('[LogoutReLogin]');

    const clearedState = await waitForPersistedAuthReset(10_000);
    if (!clearedState) {
      console.log(
        '[LogoutReLogin] Persisted auth after logout:',
        JSON.stringify(await getPersistedAuthSnapshot(), null, 2)
      );
    }
    expect(clearedState).not.toBeNull();

    setMockBehavior('telegramMeDelayMs', '4500');
    clearRequestLog();

    await triggerAuthDeepLink('e2e-logout-relogin-second-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);

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

    const secondSessionState = await waitForPersistedAuthToken(10_000);
    if (!secondSessionState) {
      console.log(
        '[LogoutReLogin] Persisted auth after re-login:',
        JSON.stringify(await getPersistedAuthSnapshot(), null, 2)
      );
    }
    expect(secondSessionState).not.toBeNull();
    expect(parsePersistedValue(secondSessionState.isOnboardedByUser)).toEqual({});
    expect(parsePersistedValue(secondSessionState.onboardingTasksByUser)).toEqual({});
    expect(parsePersistedValue(secondSessionState.hasIncompleteOnboardingByUser)).toEqual({});

    await browser.pause(1500);

    const overlayVisibleTooEarly = await isOnboardingOverlayVisible();
    if (overlayVisibleTooEarly) {
      const tree = await dumpAccessibilityTree();
      console.log('[LogoutReLogin] Overlay appeared too early. Tree:\n', tree.slice(0, 4000));
    }
    expect(overlayVisibleTooEarly).toBe(false);

    const overlayVisible = await waitForOnboardingOverlayVisible(8_000);
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

    const meCall = await waitForRequest(getRequestLog, 'GET', '/telegram/me', 10_000);
    expect(meCall).toBeDefined();
  });
});
