// @ts-nocheck
/**
 * E2E regression: onboarding overlay after logout -> re-login.
 *
 * Verifies:
 *   1. Initial login can complete onboarding and reach Home.
 *   2. Logout returns to the Welcome screen (session is cleared).
 *   3. Re-login via the auth deep-link bypass brings up the onboarding
 *      overlay at its first step, confirming the fresh session does not
 *      carry stale mid-flow onboarding state from the previous session.
 *
 * Architecture note: auth tokens live in the Rust core (not Redux-persist).
 * `applySessionToken` stores the JWT and fires `core-state:session-token-updated`
 * immediately after the token exchange, then CoreStateProvider refreshes the
 * authoritative user/profile snapshot. Routing now waits for that refreshed
 * currentUser before sending incomplete onboarding sessions to /onboarding.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  hasAppChrome,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  dismissBootCheckGateIfVisible,
  logoutViaSettings,
  performFullLogin,
  waitForOnboardingOverlayVisible,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

describe('Logout -> re-login onboarding overlay', function () {
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

  it('shows onboarding overlay with clean state after logout and re-login', async function () {
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

    // Reset core state (onboarding_completed, chat_onboarding_completed, api_key)
    // so the re-login is treated as a fresh user session. Without this,
    // the Rust core retains onboarding_completed=true from the first session
    // and the overlay would not reappear for the same mock user.
    const resetResult = await Promise.race([
      callOpenhumanRpc('openhuman.test_reset', {}),
      new Promise(resolve => setTimeout(() => resolve({ ok: false, error: 'timeout' }), 8_000)),
    ]);
    if (!resetResult.ok) {
      console.log('[LogoutReLogin] test_reset result:', JSON.stringify(resetResult));
    }

    // Reload the renderer so the CoreStateProvider picks up the fresh
    // onboarding_completed=false from the Rust core. Without this the
    // stale snapshot keeps onboarding_completed=true and the routing
    // guard never redirects to /onboarding.
    // NOTE: Do NOT clear localStorage here — that destroys the persisted
    // core mode and causes the BootCheckGate to block the entire app.
    await browser.execute(() => {
      window.location.replace('#/');
      window.location.reload();
    });
    await browser.pause(2_000);

    // The reload may surface the BootCheckGate if the core mode was lost
    // during logout. Dismiss it so the auth flow can proceed.
    await waitForWindowVisible(15_000);
    await waitForWebView(10_000);
    await dismissBootCheckGateIfVisible(12_000);
    await browser.pause(1_000);

    // ── Second login (re-login) ───────────────────────────────────────────────
    // Use the bypass deep-link path (key=auth) which skips the
    // consumeLoginToken→/telegram/login-tokens/ exchange. After the complex
    // logout→test_reset→reload cycle, the full consume flow can race against
    // waitForOAuthAuthReadiness timing — the bypass avoids that instability
    // while still exercising the core auth path (storeSession, session-token
    // event, CoreStateProvider refresh, routing guards).
    clearRequestLog();

    await triggerAuthDeepLinkBypass('e2e-logout-relogin-second');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);

    // ── Onboarding must appear for the fresh session ─────────────────────────
    // The new user has not completed onboarding, so the routed onboarding shell
    // should mount once the profile-backed core snapshot is available.
    // Allow extra time for CoreStateProvider to refresh and routing to settle.
    const overlayVisible = await waitForOnboardingOverlayVisible(40_000);
    if (!overlayVisible) {
      // Diagnostic: dump current hash, DOM text, and request log.
      const hash = await browser.execute(() => window.location.hash);
      const rootText = await browser.execute(() =>
        (document.getElementById('root')?.innerText ?? '').slice(0, 500)
      );
      console.log('[LogoutReLogin] Overlay not visible. hash=' + hash + ' rootText=' + rootText);
    }
    expect(overlayVisible).toBe(true);

    const route = await browser.execute(() => window.location.hash);
    expect(route).toMatch(/^#\/onboarding/);

    // ── Onboarding must be in clean first-step state ─────────────────────────
    // If stale mid-flow state from session 1 leaked, a later step would render
    // instead of the initial welcome step.
    const onFirstStep = await browser.execute(
      () => document.querySelector('[data-testid="onboarding-welcome-step"]') !== null
    );
    expect(onFirstStep).toBe(true);
    expect(await textExists("Hi. I'm OpenHuman.")).toBe(true);
    expect(await textExists('Get Started')).toBe(true);
  });
});
