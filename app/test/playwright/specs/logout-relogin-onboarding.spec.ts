import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  signInViaBypassUser,
  signInViaBypassUserPreservingConfig,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function readOnboardingCompleted(): Promise<boolean> {
  const completed = await callCoreRpc<boolean | { result?: boolean }>(
    'openhuman.config_get_onboarding_completed',
    {}
  );
  const value = typeof completed === 'boolean' ? completed : completed?.result;
  // Require a real boolean so a response-shape change fails loudly instead of
  // being read as "not completed".
  expect(typeof value).toBe('boolean');
  return value as boolean;
}

/**
 * Store a real (non-local) TinyHumans session with `onboarding_completed=false`
 * and open the onboarding entry route.
 *
 * `/onboarding/welcome` no longer renders anything interactive: for a real
 * session it marks onboarding complete and routes to `/chat`, so there is no
 * onboarding screen to wait for. Callers assert the landing instead.
 */
async function signInThroughOnboarding(page: Page, userId: string): Promise<void> {
  // The helper already waits for a false onboarding flag to complete and land
  // in chat. Re-entering Welcome after that would start completion a second
  // time, and its delayed navigation can pull the user away from Settings.
  await signInViaBypassUser(page, userId, { onboardingCompleted: false });
}

async function expectLandedInChat(page: Page): Promise<void> {
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/chat/);
  await expect.poll(readOnboardingCompleted, { timeout: 20_000 }).toBe(true);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = (
          window as typeof window & {
            __OPENHUMAN_CORE_STATE__?: () => { snapshot?: { onboardingCompleted?: boolean } };
          }
        ).__OPENHUMAN_CORE_STATE__?.();
        return state?.snapshot?.onboardingCompleted;
      })
    )
    .toBe(true);
  // Nothing of the onboarding stepper may linger once the user is in the app.
  await expect(page.getByTestId('onboarding-layout')).toHaveCount(0);
}

async function logoutViaSettings(page: Page): Promise<void> {
  await page.goto('/#/settings/account');
  await waitForAppReady(page);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash))
    .toMatch(/^#\/settings\/account(?:\?|$)/);
  const logout = page.getByTestId('settings-nav-logout');
  await expect(logout).toBeVisible();
  await logout.click();
  await expect(page.getByTestId('welcome-cta-tinyhumans')).toBeVisible({ timeout: 20_000 });
  // A core RPC response only confirms that persistence was updated. Wait for
  // the browser's CoreStateProvider to observe that signed-out snapshot before
  // asserting the public route, otherwise a preceding authenticated snapshot
  // can win the reload race in a busy serial CI lane.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = (
          window as typeof window & {
            __OPENHUMAN_CORE_STATE__?: () => { snapshot?: { sessionToken?: string | null } };
          }
        ).__OPENHUMAN_CORE_STATE__?.();
        return Boolean(state?.snapshot?.sessionToken);
      })
    )
    .toBe(false);
}

test.describe('Logout -> re-login onboarding overlay', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await bootRuntimeReadyGuestPage(page);
  });

  test('same-account completed onboarding survives UI logout and re-login', async ({ page }) => {
    const userId = 'pw-logout-relogin-completed-user';
    await signInViaBypassUser(page, userId);
    await expectLandedInChat(page);

    await logoutViaSettings(page);
    await expect.poll(readOnboardingCompleted).toBe(true);
    await expect(page.getByTestId('onboarding-layout')).toHaveCount(0);

    await signInViaBypassUserPreservingConfig(page, userId);
    await page.goto('/#/onboarding/welcome');
    await waitForAppReady(page);
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/chat/);
    await expect.poll(readOnboardingCompleted).toBe(true);
    await expect(page.getByTestId('onboarding-layout')).toHaveCount(0);
  });

  test('re-login after logout re-runs onboarding from clean state and lands in chat', async ({
    page,
  }) => {
    // First login: onboarding resolves on its own for a TinyHumans session.
    await signInThroughOnboarding(page, 'pw-logout-relogin-user');
    await expectLandedInChat(page);

    // Logout drops the session and shows the signed-out Welcome screen, not a
    // stale onboarding or chat surface.
    await logoutViaSettings(page);
    await expect(page.getByTestId('welcome-cta-tinyhumans')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('onboarding-layout')).toHaveCount(0);

    // Re-login with onboarding reset: the flow must run again from the start
    // (not be skipped by leftover state from the first session, and not wedge
    // on a screen) and end in chat with the flag set again.
    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
    expect(await readOnboardingCompleted()).toBe(false);

    await signInThroughOnboarding(page, 'pw-logout-relogin-user');
    await expectLandedInChat(page);
  });
});
