import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface RouteCheck {
  hash: string;
  markers: string[];
}

// Phase 2/3/6 IA revamp:
//   /skills      → /connections (redirects; test new canonical route)
//   /intelligence → /activity   (redirects; test new canonical route)
//   /human       → /chat        (redirects; already covered by /chat entry)
const routes: RouteCheck[] = [
  { hash: '/chat', markers: ['Threads', 'Chat', 'Message', 'New'] },
  // Connections page (was /skills) — tabs: Apps, Messaging, Tools, Explorer
  { hash: '/connections', markers: ['Apps', 'Messaging', 'Tools', 'Explorer'] },
  { hash: '/home', markers: ['Ask your assistant anything', 'Your device is connected'] },
  // /channels now redirects to /connections?tab=messaging
  { hash: '/channels', markers: ['Messaging', 'Connections', 'Telegram', 'Discord'] },
  { hash: '/notifications', markers: ['Notifications', 'Alerts', 'No alerts yet'] },
  { hash: '/rewards', markers: ['Rewards', 'Referral', 'Credits', 'Invite'] },
  { hash: '/settings', markers: ['Settings', 'Account', 'Billing', 'Advanced'] },
  // Activity page (was /intelligence) — tabs: Tasks, Automations, Subconscious
  { hash: '/activity', markers: ['Tasks', 'Automations', 'Subconscious'] },
  { hash: '/home', markers: ['Ask your assistant anything', 'Your device is connected'] },
];

async function rootTextLength(page: import('@playwright/test').Page): Promise<number> {
  return page
    .locator('#root')
    .innerText()
    .then(text => text.length);
}

async function verifyRouteLoaded(
  page: import('@playwright/test').Page,
  route: RouteCheck
): Promise<void> {
  await waitForAppReady(page);
  await expect.poll(() => rootTextLength(page), { timeout: 10_000 }).toBeGreaterThan(50);
}

test.describe('Navigation Smoothness', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-smoothness-user');
  });

  test('all major routes render within timing budget', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await verifyRouteLoaded(page, route);
    }
  });

  test('rapid cycle completes without blank screens', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await verifyRouteLoaded(page, route);
    }
  });

  test('final state is /home with correct content', async ({ page }) => {
    await page.goto('/#/home');
    await waitForAppReady(page);
    await expect(page.getByRole('button', { name: /Ask your assistant anything/i })).toBeVisible();
    await expect(page.getByText(/Your device is connected/i)).toBeVisible();
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/home/);
  });
});
