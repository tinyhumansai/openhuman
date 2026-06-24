import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface RouteEntry {
  route: string;
  /** Expected hash after any redirect. Defaults to the route itself. */
  expectedHash?: string;
}

// Phase 2/3/6 IA revamp routes.
// Back-compat redirects are included so the router redirect itself is tested.
//   /human       → renders the Human surface (first-class route, restored)
//   /skills      → /connections (Phase 2)
//   /activity    → /settings/notifications (Phase 6)
//   /intelligence → /settings/notifications (Phase 6)
//   /home        → /chat (Home folded into the unified two-panel chat surface)
const ROUTES: RouteEntry[] = [
  { route: '/home', expectedHash: '/chat' }, // back-compat redirect (Home → chat)
  { route: '/human' }, // first-class route again (no longer redirects to /chat)
  { route: '/chat' },
  { route: '/connections' },
  { route: '/skills', expectedHash: '/connections' }, // back-compat redirect
  { route: '/activity', expectedHash: '/settings/notifications' }, // back-compat redirect
  { route: '/intelligence', expectedHash: '/settings/notifications' }, // back-compat redirect
  { route: '/rewards' },
  // Desktop Settings is a modal overlay (the backgroundLocation pattern): the
  // /settings index redirects to the first panel and the modal renders on top
  // of the /chat fallback backdrop.
  { route: '/settings', expectedHash: '/settings/account' },
];

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-user');
  });

  for (const { route, expectedHash } of ROUTES) {
    const landing = expectedHash ?? route;
    test(`renders ${route}`, async ({ page }) => {
      await page.goto(`/#${route}`);
      await waitForAppReady(page);

      // After redirects the hash should begin with the final landing path.
      await expect
        .poll(async () => page.evaluate(() => window.location.hash))
        .toMatch(new RegExp(`^#${landing.replace('/', '\\/')}`));
      await expect
        .poll(async () => {
          const text = await page.locator('#root').innerText();
          return text.trim().length;
        })
        .toBeGreaterThan(50);
    });
  }
});
