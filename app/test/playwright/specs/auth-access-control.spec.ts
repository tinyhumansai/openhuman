import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

// Declared locally, matching every other spec in this directory
// (`chat-harness-send-stream`, `runtime-picker-login`, …). It was used at
// `mockRequests` below without ever being declared, which is where this file's
// two pre-existing `tsc -p test/tsconfig.e2e.json` errors came from.
interface MockRequest {
  method: string;
  url: string;
  body?: string;
}

/** `AuthStateResponse` (`crates/openhuman-core/src/security/credentials/responses.rs`). */
interface AuthState {
  isAuthenticated: boolean;
  userId?: string | null;
  credential?: string;
}

async function authState(): Promise<AuthState> {
  const raw = (await callCoreRpc<Record<string, unknown>>('openhuman.auth_get_state', {})) as
    | AuthState
    | { value?: AuthState; is_authenticated?: boolean; user_id?: string | null };
  const state = 'value' in raw && raw.value ? raw.value : raw;
  const wire = state as AuthState & { is_authenticated?: boolean; user_id?: string | null };
  return {
    isAuthenticated: Boolean(wire.isAuthenticated ?? wire.is_authenticated),
    userId: wire.userId ?? wire.user_id,
    credential: wire.credential,
  };
}

async function gotoSettingsRoute(page: Page, hash: string): Promise<void> {
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function mockRequests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await response.json()) as { data?: MockRequest[] };
  return Array.isArray(payload.data) ? payload.data : [];
}

async function waitForMockRequest(method: string, pathFragment: string, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let delay = 200;
  while (Date.now() < deadline) {
    const match = (await mockRequests()).find(
      request => request.method === method && request.url.includes(pathFragment)
    );
    if (match) return match;
    await new Promise(resolve => setTimeout(resolve, delay));
    delay = Math.min(delay * 1.5, 1_000);
  }
  return null;
}

test.describe('Auth & Access Control', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('authenticated sign-in reaches home', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-access-token');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/(home|chat)/);
    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();
  });

  test('re-authenticating with a second bypass user keeps the user in-app', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-access-first');
    await dismissWalkthroughIfPresent(page);

    await signInViaBypassUser(page, 'pw-auth-access-second');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toMatch(/^#\/(home|chat)/);
    await expect
      .poll(async () => {
        const requests = await mockRequests();
        return requests.filter(
          request => request.method === 'GET' && request.url.includes('/auth/me')
        ).length;
      })
      .toBeGreaterThanOrEqual(2);
  });

  // Was `test.skip(true, 'shared web auth bootstrap is unstable … can fall
  // back to onboarding instead of home')`. The instability was in asserting a
  // *route*; the claim in the title is about the *request log*, which the two
  // tests above already read reliably. Asserting the claim directly needs no
  // route settle at all. (matrix 1.3.1)
  test('second-device bypass token is accepted without hitting token consume', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-second-device');

    await expect(await waitForMockRequest('GET', '/auth/me')).toBeTruthy();

    // The point of a bypass credential: the core installs it directly through
    // `auth.set_credential` and never redeems it at the backend. A regression
    // that routed bypass sign-in back through the consume endpoint would send
    // an unredeemable token to `/auth/login-token/consume`, get a 401, and
    // log the user straight back out.
    const consumeCalls = (await mockRequests()).filter(
      request => request.method === 'POST' && request.url.includes('/auth/login-token/consume')
    );
    expect(consumeCalls, 'bypass sign-in must not redeem a login token').toHaveLength(0);

    // The browser bypass credential deliberately does not become a hosted
    // session in `auth_get_state`; the authenticated shell and `/auth/me`
    // request above are the authoritative signals for this lane.
  });

  // Was `test.skip(true, 'shared web auth/bootstrap helper is not stable
  // enough yet for logout coverage without crashing the standalone core
  // lane')`. That reason is stale: `signInViaBypassUser` already calls
  // `auth_clear_session` on this same core in every `beforeEach`, so clearing
  // a session in this lane is demonstrably survivable. (matrix 1.4.1)
  //
  // Asserts through `auth.get_state` rather than a text search for "Welcome":
  // the WD counterpart asserted `onWelcome || !localStorage['persist:auth']`,
  // and since there is no `auth` reducer that key is always null, so its
  // disjunction was a tautology that passed whether or not logout worked.
  test('logout via settings clears the session and returns to welcome', async ({ page }) => {
    await signInViaBypassUser(page, 'pw-auth-logout-user');

    await gotoSettingsRoute(page, '/settings/account');
    await page.getByTestId('settings-nav-logout').click();

    // Mechanism 1: the core no longer holds a credential. This is the half
    // that matters for security — a UI that routes to Welcome while the core
    // keeps authenticating is the regression worth catching.
    await expect
      .poll(async () => (await authState()).isAuthenticated, { timeout: 15_000 })
      .toBe(false);
    const cleared = await authState();
    expect(cleared.credential, 'no credential should back a signed-out state').toBeUndefined();
    expect(cleared.userId ?? null).toBeNull();

    // Mechanism 2: the shell actually leaves the authenticated surface. Two
    // independent signals, so a partial regression fails rather than passing
    // on whichever half still works.
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
      .not.toMatch(/^#\/(chat|home|settings)/);
  });

  // DELETED rather than unskipped: `auth-expired event signs the user out and
  // lands on welcome`.
  //
  // Its skip reason was substantive, not flakiness: *"web Playwright lane uses
  // a local/bypass session that intentionally ignores auth-expired handling"*.
  // Every sign-in helper in this lane installs a bypass credential through
  // `auth_store_session`, and that path deliberately does not arm expiry
  // handling, so the behaviour named in the title cannot occur here however
  // the test is written. Leaving it as an unconditional skip made matrix row
  // 1.4.3 read as serviced.
  //
  // Auth expiry / server-side revocation is covered instead at the two layers
  // where it is real:
  //   * WD  — `app/test/e2e/specs/auth-access-control.spec.ts`,
  //           `revoked session auto-logs out the user` (a real session, mock
  //           `session: 'revoked'` → 401).
  //   * RU  — `crates/openhuman-tinyhumans/src/session/manager_tests.rs`,
  //           the 401-clears-manager-cache-and-identity-slot cases.
  //
  // DELETED rather than unskipped: `billing dashboard handoff remains
  // available for authenticated users` — duplicates
  // `app/test/playwright/specs/settings-account-preferences.spec.ts`, which
  // already navigates `/settings/billing` and asserts the handoff. Billing is
  // matrix section 3, not this file's section 1.
});
