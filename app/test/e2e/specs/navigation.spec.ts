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
import { hasAppChrome, waitForTestId } from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash, waitForHomePage } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-navigation';

interface Route {
  hash: string;
  /** Min character count we expect in the rendered React tree after the
   * route mounts. A truly-blank screen surfaces as <100 chars of text. */
  minChars?: number;
  /** A `data-testid` that must be in the DOM once the route has mounted. */
  readyTestId?: string;
}

// Phase 2/3/6 IA revamp:
//   /home        → /chat        (Phase 6 — /home is now the merged chat surface)
//   /human       → renders the Human surface (first-class tab)
//   /skills      → /connections (Phase 2 — back-compat redirect)
//   /intelligence → /settings/notifications (Phase 3 — back-compat redirect)
//   /activity     → /settings/notifications (back-compat redirect)
// Note: /home and /activity are intentionally omitted here because AppRoutes.tsx
// now redirects them (/home → /chat, /activity → /settings/notifications).
// navigateViaHash settles on the redirect target, so keeping them in ROUTES
// would fail the `^#<hash>` assertion (actual hash is the redirect destination).
const ROUTES: Route[] = [
  { hash: '/chat' },
  { hash: '/connections' },
  { hash: '/settings' },
  { hash: '/flows' },
  // Memory (v2) is a sub-page of Connections; the retired `/brain` route
  // redirects here, so assert the canonical destination (a redirecting hash
  // would settle on its target and fail the `^#<hash>` match, same reasoning
  // as /home above). The Memory page mounts with its chip bar whatever the
  // engine state, so its root testid is a stable ready signal.
  { hash: '/connections?tab=brain', readyTestId: 'memory-page' },
];

async function rootTextLength(): Promise<number> {
  return (await browser.execute(
    () => (document.getElementById('root')?.innerText ?? '').length
  )) as number;
}

describe('Navigation', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('app chrome stays visible', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('lands on /home after onboarding', async () => {
    await waitForAppReady(10_000);
    let homeText = await waitForHomePage(15_000);
    if (!homeText) {
      // resetApp may have landed on /chat instead of /home; navigate explicitly.
      await navigateViaHash('/home');
      await waitForAppReady(10_000);
      homeText = await waitForHomePage(15_000);
    }
    expect(homeText).toBeTruthy();
  });

  for (const route of ROUTES) {
    it(`renders ${route.hash}`, async () => {
      await navigateViaHash(route.hash);
      await waitForAppReady(10_000);

      const hash = await browser.execute(() => window.location.hash);
      // Escape the hash: `?` in `/connections?tab=brain` is a regex quantifier.
      const escaped = route.hash.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      expect(hash).toMatch(new RegExp(`^#${escaped}`));

      // waitForTestId is tauri-driver only; Mac2 keeps the hash + char checks.
      if (route.readyTestId && isTauriDriver()) {
        await waitForTestId(route.readyTestId, 15_000);
      }

      const chars = await rootTextLength();
      expect(chars).toBeGreaterThan(route.minChars ?? 50);
    });
  }
});
