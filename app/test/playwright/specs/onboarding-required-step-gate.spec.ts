import { expect, type Page, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

/**
 * Onboarding cannot be bypassed while `onboarding_completed` is false.
 *
 * This file used to pin the `/onboarding/runtime-choice` step (cloud
 * pre-selected, Continue enabled, options mutually exclusive). That step is
 * gone: the identity question is asked once on the Welcome screen, and
 * `/onboarding/welcome` now only routes. What the old spec was really
 * protecting is that the app-shell onboarding gate (`App.tsx`,
 * `[onboarding-gate]`) holds a user in onboarding until it is genuinely done,
 * and that the required steps are passed through rather than skipped. That is
 * what is asserted here:
 *
 *   - local session: every in-app route bounces to the first of the three
 *     custom steps, the steps run inference -> search -> vault in order, and
 *     `onboarding_completed` stays false until the last one is finished.
 *   - TinyHumans session: nothing is left to configure, so the gate resolves
 *     itself (welcome marks onboarding complete and routes to chat) instead of
 *     leaving the user on a dead screen.
 *   - the retired runtime-choice route cannot be reached.
 *
 * `onboarding-modes.spec.ts` walks the happy paths through to completion.
 */

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  // Deliberately NOT swallowed. A failed reset leaves shared mock state from a
  // previous test, which surfaces as an unrelated assertion failure later.
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
  if (!response.ok) {
    throw new Error(`mock admin reset failed with HTTP ${response.status}`);
  }
}

const hash = (page: Page) => page.evaluate(() => window.location.hash);

function sessionToken(userId: string, signature: string): string {
  const payload = Buffer.from(
    JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  return `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.${signature}`;
}

/**
 * Store a session, reset `onboarding_completed=false` and reload the app so its
 * core snapshot reflects both. The "local" signature is what marks the offline
 * "Set it up myself" session (`isLocalSessionToken`); any other is TinyHumans.
 */
async function bootWithSession(
  page: Page,
  userId: string,
  kind: 'local' | 'tinyhumans',
  startHash: string
): Promise<void> {
  await resetMock();
  await bootRuntimeReadyGuestPage(page);
  await callCoreRpc('openhuman.auth_store_session', {
    token: sessionToken(userId, kind === 'local' ? 'local' : 'sig'),
    ...(kind === 'local'
      ? { user: { _id: 'local', id: 'local', name: 'Local User', email: 'local@openhuman.local' } }
      : {}),
  });
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto(`/#${startHash}`);
  await page.reload();
  await waitForAppReady(page);
}

async function readOnboardingCompleted(): Promise<boolean> {
  const completed = await callCoreRpc<boolean | { result?: boolean }>(
    'openhuman.config_get_onboarding_completed',
    {}
  );
  const value = typeof completed === 'boolean' ? completed : completed?.result;
  // `Boolean(completed?.result)` alone would turn a malformed response into
  // `false` and quietly satisfy a "still incomplete" assertion. Require a real
  // boolean so a shape change fails loudly.
  expect(typeof value).toBe('boolean');
  return value as boolean;
}

const nextButton = (page: Page) => page.getByTestId('onboarding-next-button');

async function expectOnFirstCustomStep(page: Page): Promise<void> {
  await expect
    .poll(() => hash(page), { timeout: 20_000 })
    .toMatch(/^#\/onboarding\/custom\/inference/);
  await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible({
    timeout: 20_000,
  });
}

test.describe('Onboarding — cannot be bypassed while incomplete', () => {
  for (const target of ['/chat', '/settings', '/human']) {
    test(`local session: ${target} bounces to the first custom step`, async ({ page }) => {
      await bootWithSession(page, 'pw-gate-local-bounce', 'local', target);

      await expectOnFirstCustomStep(page);
      expect(await readOnboardingCompleted()).toBe(false);
    });
  }

  test('local session: the three custom steps run in order and do not complete onboarding early', async ({
    page,
  }) => {
    await bootWithSession(page, 'pw-gate-local-steps', 'local', '/onboarding/welcome');

    await expectOnFirstCustomStep(page);
    expect(await readOnboardingCompleted()).toBe(false);

    await nextButton(page).click();
    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible({
      timeout: 20_000,
    });
    await expect(page.getByTestId('onboarding-custom-inference-step')).toHaveCount(0);
    expect(await readOnboardingCompleted()).toBe(false);

    await nextButton(page).click();
    await expect(page.getByTestId('onboarding-custom-vault-step')).toBeVisible({ timeout: 20_000 });
    // Still in onboarding on the final step, until it is finished.
    expect(await hash(page)).toMatch(/^#\/onboarding\/custom\/vault/);
    expect(await readOnboardingCompleted()).toBe(false);

    // Voice, OAuth and embeddings are retired as wizard steps.
    for (const retired of ['voice', 'oauth', 'embeddings']) {
      await expect(page.getByTestId(`onboarding-custom-${retired}-step`)).toHaveCount(0);
    }
  });

  test('local session: Back returns to the previous required step', async ({ page }) => {
    await bootWithSession(page, 'pw-gate-local-back', 'local', '/onboarding/welcome');

    await expectOnFirstCustomStep(page);
    await nextButton(page).click();
    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible({
      timeout: 20_000,
    });

    await page.getByRole('button', { name: /Back/ }).click();
    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible({
      timeout: 20_000,
    });
    expect(await readOnboardingCompleted()).toBe(false);
  });

  test('the retired runtime-choice route is unreachable and falls through to the gate', async ({
    page,
  }) => {
    await bootWithSession(page, 'pw-gate-retired-route', 'local', '/onboarding/runtime-choice');

    await expectOnFirstCustomStep(page);
    await expect(page.getByTestId('onboarding-runtime-choice-step')).toHaveCount(0);
    expect(await readOnboardingCompleted()).toBe(false);
  });

  test('TinyHumans session: the gate resolves itself and lands in chat', async ({ page }) => {
    // Nothing is left to configure for a managed session, so a user held at the
    // gate must not be stranded on a screen that needs input.
    await bootWithSession(page, 'pw-gate-tinyhumans', 'tinyhumans', '/settings');

    await expect.poll(() => hash(page), { timeout: 20_000 }).toMatch(/^#\/chat/);
    await expect.poll(readOnboardingCompleted, { timeout: 20_000 }).toBe(true);
    await expect(page.getByTestId('onboarding-layout')).toHaveCount(0);
  });
});
