import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface RouteEntry {
  route: string;
  /** Expected hash after any redirect. Defaults to the route itself. */
  expectedHash?: string;
  readyTestId?: string;
}

// Phase 2/3/6 IA revamp routes.
// Back-compat redirects are included so the router redirect itself is tested.
//   /human       → renders the Human surface (first-class route)
//   /skills      → /connections (Phase 2)
//   /activity    → /settings/account (Phase 6)
//   /intelligence → /settings/account (Phase 6)
//   /home        → /chat (Home folded into the unified two-panel chat surface)
const ROUTES: RouteEntry[] = [
  { route: '/home', expectedHash: '/chat' }, // back-compat redirect (Home → chat)
  { route: '/human' }, // first-class route (dedicated mascot stage)
  { route: '/chat' },
  { route: '/connections' },
  { route: '/flows' },
  { route: '/connections?tab=brain', readyTestId: 'memory-page' },
  { route: '/brain', expectedHash: '/connections?tab=brain', readyTestId: 'memory-page' },
  { route: '/skills', expectedHash: '/connections' }, // back-compat redirect
  { route: '/activity', expectedHash: '/settings/account' }, // back-compat redirect
  { route: '/intelligence', expectedHash: '/settings/account' }, // back-compat redirect
  // Settings is a routed page and the /settings index resolves to Account.
  { route: '/settings', expectedHash: '/settings/account' },
];

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-user');
  });

  for (const { route, expectedHash, readyTestId } of ROUTES) {
    const landing = expectedHash ?? route;
    test(`renders ${route}`, async ({ page }) => {
      await page.goto(`/#${route}`);
      await waitForAppReady(page);

      // After redirects the hash should begin with the final landing path.
      const escapedLanding = landing.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const minimumTextLength = route === '/human' ? 20 : 50;
      await expect
        .poll(async () => page.evaluate(() => window.location.hash))
        .toMatch(new RegExp(`^#${escapedLanding}`));
      if (expectedHash?.includes('?')) {
        await expect.poll(() => page.evaluate(() => window.location.hash)).toBe(`#${expectedHash}`);
      }
      if (readyTestId) await expect(page.getByTestId(readyTestId)).toBeVisible();
      await expect
        .poll(async () => {
          const text = await page.locator('#root').innerText();
          return text.trim().length;
        })
        // Human is intentionally sparse; native-equivalent app routes must
        // retain the broader nonblank-content threshold.
        .toBeGreaterThan(minimumTextLength);
    });
  }
});
