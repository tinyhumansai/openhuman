/**
 * Screenshot capture of every onboarding screen, light and dark.
 *
 * Not an assertion suite — it walks the flow and writes a PNG per screen so a
 * human can eyeball the restyle. Named `zz-` so it runs last.
 */
import { type Page, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

/**
 * A local ("Continue Locally") session token: three segments whose last is
 * literally `local`. See app/src/utils/localSession.ts.
 *
 * The capture has to use one. An authenticated managed session now completes
 * onboarding on sight -- `WelcomePage` calls completeAndExit() and the gate
 * routes to /chat -- so the three self-hosted steps are only reachable from a
 * local session.
 */
function localSessionToken(): string {
  const enc = (o: object) =>
    Buffer.from(JSON.stringify(o)).toString('base64url').replace(/=+$/, '');
  const now = Math.floor(Date.now() / 1000);
  return [
    enc({ alg: 'none', typ: 'JWT' }),
    enc({ sub: 'local', user_id: 'local', iat: now, exp: now + 31536000 }),
    'local',
  ].join('.');
}

const OUT = 'test-results/onboarding-screens';

async function bootIntoOnboarding(page: Page, userId: string): Promise<void> {
  await bootRuntimeReadyGuestPage(page);
  await callCoreRpc('openhuman.auth_store_session', {
    token: localSessionToken(),
    userId,
    user: { _id: 'local', id: 'local', name: 'Local User', email: 'local@openhuman.local' },
  });
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/custom/inference');
  await page.reload();
  await waitForAppReady(page);
}

async function go(page: Page, hash: string, stepTestId?: string): Promise<void> {
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
  if (stepTestId) {
    await page.getByTestId(stepTestId).waitFor({ state: 'visible', timeout: 30_000 });
  }
}

async function setTheme(page: Page, dark: boolean): Promise<void> {
  await page.evaluate(d => {
    document.documentElement.classList.toggle('dark', d);
  }, dark);
  await page.waitForTimeout(250);
}

async function shoot(page: Page, name: string): Promise<void> {
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: true });
}

test.describe('Onboarding screens', () => {
  test('captures every screen in light and dark', async ({ page }) => {
    test.setTimeout(180_000);
    await bootIntoOnboarding(page, 'screens-user');

    for (const dark of [false, true]) {
      const mode = dark ? 'dark' : 'light';

      await go(page, '/onboarding/custom/inference', 'onboarding-custom-inference-step');
      await setTheme(page, dark);
      await shoot(page, `01-inference-${mode}`);
      await shoot(page, `02-inference-detail-${mode}`);

      await go(page, '/onboarding/custom/search', 'onboarding-custom-search-step');
      await setTheme(page, dark);
      await shoot(page, `03-search-${mode}`);
      await shoot(page, `04-search-detail-${mode}`);

      await go(page, '/onboarding/custom/vault', 'onboarding-custom-vault-step');
      await setTheme(page, dark);
      await shoot(page, `05-vault-${mode}`);
      await shoot(page, `06-vault-detail-${mode}`);
    }

    // Narrow viewport — the p-10 -> p-6 sm:p-8 change targeted row wrapping.
    await page.setViewportSize({ width: 400, height: 900 });
    await go(page, '/onboarding/custom/inference', 'onboarding-custom-inference-step');
    await setTheme(page, false);
    await shoot(page, '07-inference-narrow-400px');
    await go(page, '/onboarding/custom/search', 'onboarding-custom-search-step');
    await shoot(page, '08-search-narrow-400px');
  });
});
