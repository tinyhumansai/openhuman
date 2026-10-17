// @ts-nocheck
/**
 * E2E regression: onboarding overlay after logout -> re-login.
 *
 * Verifies:
 *   1. Initial login can complete onboarding and reach Home.
 *   2. Logout returns to the Welcome screen (session is cleared).
 *   3. Re-login via the auth deep-link bypass returns to chat without
 *      restarting onboarding for the same completed account.
 *
 * Architecture note: auth tokens live in the Rust core (not Redux-persist).
 * `applySessionToken` stores the JWT and fires `core-state:session-token-updated`
 * immediately after the token exchange, then CoreStateProvider refreshes the
 * authoritative user/profile snapshot. Routing now waits for that refreshed
 * currentUser before sending incomplete onboarding sessions to /onboarding.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { hasAppChrome, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { triggerAuthLoopbackBypass } from '../helpers/loopback-auth-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  dismissBootCheckGateIfVisible,
  logoutViaSettings,
  performFullLogin,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

describe('Logout -> re-login onboarding state', function () {
  // Suite-level timeout — covers all hooks and tests. The full flow
  // (resetApp + first login + logout + test_reset + reload + re-login)
  // can take 60-90s, well over the default 30s.
  this.timeout(180_000);

  before(async () => {
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

  it('keeps completed onboarding after logout and re-login', async function () {
    const hasChrome = await hasAppChrome();
    expect(hasChrome).toBe(true);

    // ── First login: complete onboarding and reach Home ──────────────────────
    clearRequestLog();
    resetMockBehavior();
    await performFullLogin('e2e-logout-relogin-first-token', '[LogoutReLogin]');

    // Let post-onboarding routing guards settle before navigating to Settings.
    await browser.pause(2_000);

    // ── Logout ────────────────────────────────────────────────────────────────
    await logoutViaSettings('[LogoutReLogin]');
    // logoutViaSettings confirms "Welcome" is visible — the session is cleared.

    // Onboarding completion belongs to the user. Re-authenticate the same
    // account and verify logout did not send it through setup again.
    await browser.execute(() => {
      window.location.replace('#/');
      window.location.reload();
    });
    await browser.pause(2_000);
    await waitForWindowVisible(15_000);
    await waitForWebView(10_000);
    await dismissBootCheckGateIfVisible(12_000);

    clearRequestLog();
    await triggerAuthLoopbackBypass('e2e-logout-relogin-first-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);

    await browser.waitUntil(
      async () =>
        (await browser.execute(() => window.location.hash.startsWith('#/chat'))) as boolean,
      { timeout: 30_000, interval: 250, timeoutMsg: 'Re-login did not return to chat' }
    );
    const onboarding = await callOpenhumanRpc('openhuman.config_get_onboarding_completed', {});
    expect(onboarding.ok).toBe(true);
    expect(onboarding.result.result).toBe(true);
    expect(
      await browser.execute(
        () => document.querySelector('[data-testid="onboarding-layout"]') !== null
      )
    ).toBe(false);
  });
});
