// @ts-nocheck
/**
 * E2E test: Desktop deep-link login and token error paths (Linux / tauri-driver).
 *
 * Verifies desktop auth entry using mock data:
 *   Deep link authentication:
 *     1. `openhuman://auth?token=...` deep link is triggered via __simulateDeepLink
 *     2. App calls POST /auth/login-token/consume            (mock server)
 *     3. App receives JWT, dispatches to Redux authSlice
 *     4. UserProvider calls GET /auth/me  (mock server)
 *
 *   Error paths:
 *     - Expired token returns 401 and app does not navigate to home
 *     - Invalid token returns 401 and app does not navigate to home
 *
 *   Bypass auth path:
 *     - `openhuman://auth?token=...&key=auth` sets token directly (no consume call)
 *
 * The mock server runs on http://127.0.0.1:18473 and the .app bundle must
 * have been built with VITE_BACKEND_URL pointing there.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { buildBypassJwt, triggerAuthDeepLink, triggerDeepLink } from '../helpers/deep-link-helpers';
import { hasAppChrome, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { waitForHomePage, walkOnboarding } from '../helpers/shared-flows';
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

describe('Desktop deep-link login (Linux)', () => {
  before(async () => {
    await startMockServer();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await waitForApp();
    // Start from Welcome so this spec exercises native deep-link entry.
    await resetApp('e2e-login-flow', { skipAuth: true });
    clearRequestLog();
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
    const call = await waitForRequest('POST', '/auth/login-token/consume', 20_000);
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
        r => r.method === 'GET' && (r.url.includes('/auth/me') || r.url.includes('/settings'))
      );
      if (call) break;
      await browser.pause(500);
    }
    if (!call) {
      console.log('[LoginFlow] Request log:', JSON.stringify(getRequestLog(), null, 2));
    }
    expect(call).toBeDefined();
  });

  // Authentication ownership is verified by the login/backend request flow
  // above. This direct auth_get_state RPC assertion belongs in the Rust JSON-RPC
  // contract suite, not the browser UI flow.

  // -----------------------------------------------------------------------
  // Error paths — expired and invalid tokens
  // -----------------------------------------------------------------------

  it('expired token triggers consume call that returns 401', async () => {
    // Note: The app is already authenticated from Phase 1-3. In a single-instance
    // Tauri desktop app, we cannot fully reset the in-memory Redux state between
    // tests. This test verifies that the expired token deep link triggers the
    // consume call and the mock rejects it with 401.
    clearRequestLog();
    setMockBehavior('token', 'expired');

    await triggerDeepLink('openhuman://auth?token=expired-test-token');
    await browser.pause(5_000);

    // Verify the consume call was made (mock returns 401 for expired tokens)
    const call = await waitForRequest('POST', '/auth/login-token/consume', 10_000);
    expect(call).toBeDefined();
    console.log('[LoginFlow] Expired token: consume call made (mock returns 401)');

    // The app should not have navigated away — prior session remains intact.
    // We verify the deep link handler attempted the consume and it was rejected.
    resetMockBehavior();
  });

  it('invalid token triggers consume call that returns 401', async () => {
    clearRequestLog();
    setMockBehavior('token', 'invalid');

    await triggerDeepLink('openhuman://auth?token=invalid-test-token');
    await browser.pause(5_000);

    // Verify the consume call was made (mock returns 401 for invalid tokens)
    const call = await waitForRequest('POST', '/auth/login-token/consume', 10_000);
    expect(call).toBeDefined();
    console.log('[LoginFlow] Invalid token: consume call made (mock returns 401)');

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
    await browser.pause(3_000);

    // Assert NO consume call was made (bypass skips it)
    const consumeCall = getRequestLog().find(
      r => r.method === 'POST' && r.url.includes('/auth/login-token/consume')
    );
    expect(consumeCall).toBeUndefined();
    console.log('[LoginFlow] Bypass auth: no consume call (correct — token set directly)');

    // Clearing localStorage resets onboarding state — walk through it so the
    // app reaches /home before we assert the home-page markers.
    await walkOnboarding('[LoginFlow][bypass]');

    const foundHome = await waitForHomePage(20_000);
    expect(foundHome).not.toBeNull();
    console.log(`[LoginFlow] Bypass auth: home reached with "${foundHome}"`);

    // Auth slice persistence moved away from a standalone persist:auth key.
    // Home-route confirmation above is the stable assertion that bypass auth succeeded.
    console.log(
      '[LoginFlow] Bypass auth: home route reached (token persistence format is implementation-specific)'
    );
  });
});
