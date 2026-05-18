// @ts-nocheck
/**
 * Navigation spec — drives every top-level route the BottomTabBar exposes
 * and asserts each one actually renders.
 *
 * After `resetApp(...)` the user is logged in and onboarded. From there we
 * navigate via the hash router (the same primitive `cron-jobs-flow.spec.ts`
 * uses) and confirm:
 *
 *   - `window.location.hash` updates to the requested route
 *   - The React tree under `#root` has rendered content for it
 *
 * Catches regressions where a tab loads to a blank screen, errors out, or
 * the BottomTabBar / router silently no-ops.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash, waitForHomePage } from '../helpers/shared-flows';

const USER_ID = 'e2e-navigation';

interface Route {
  hash: string;
  /** Min character count we expect in the rendered React tree after the
   * route mounts. A truly-blank screen surfaces as <100 chars of text. */
  minChars?: number;
}

const ROUTES: Route[] = [
  { hash: '/home' },
  { hash: '/human' },
  { hash: '/chat' },
  { hash: '/skills' },
  { hash: '/intelligence' },
  { hash: '/rewards' },
  { hash: '/settings' },
];

async function rootTextLength(): Promise<number> {
  return (await browser.execute(
    () => (document.getElementById('root')?.innerText ?? '').length
  )) as number;
}

describe('Navigation', () => {
  before(async () => {
    await waitForApp();
    await resetApp(USER_ID);
  });

  it('app chrome stays visible', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('lands on /home after onboarding', async () => {
    await waitForAppReady(10_000);
    const homeText = await waitForHomePage(15_000);
    expect(homeText).toBeTruthy();
  });

  for (const route of ROUTES) {
    it(`renders ${route.hash}`, async () => {
      await navigateViaHash(route.hash);
      await waitForAppReady(10_000);

      const hash = await browser.execute(() => window.location.hash);
      expect(hash).toMatch(new RegExp(`^#${route.hash}`));

      const chars = await rootTextLength();
      expect(chars).toBeGreaterThan(route.minChars ?? 50);
    });
  }
});
