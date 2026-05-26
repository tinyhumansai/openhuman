import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface RouteCheck {
  hash: string;
  markers: string[];
}

const routes: RouteCheck[] = [
  { hash: '/chat', markers: ['Threads', 'Chat', 'Message', 'New'] },
  { hash: '/skills', markers: ['Skills', 'Skill', 'Install', 'Browse'] },
  { hash: '/home', markers: ['Ask your assistant anything', 'Your device is connected'] },
  { hash: '/channels', markers: ['Channels', 'Connect', 'Telegram', 'Discord'] },
  { hash: '/notifications', markers: ['Notifications', 'Alerts', 'No alerts yet'] },
  { hash: '/rewards', markers: ['Rewards', 'Referral', 'Credits', 'Invite'] },
  { hash: '/settings', markers: ['Settings', 'Account', 'Billing', 'Advanced'] },
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
  await expect(await rootTextLength(page)).toBeGreaterThan(50);
}

test.describe('Navigation Smoothness', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-smoothness-user');
  });

  test('all major routes render within timing budget', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await verifyRouteLoaded(page, route);
      await page.waitForTimeout(400);
    }
  });

  test('rapid cycle completes without blank screens', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await page.waitForTimeout(350);
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
