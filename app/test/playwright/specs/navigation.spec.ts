import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

const routes = ['/home', '/human', '/chat', '/skills', '/intelligence', '/rewards', '/settings'];

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-user');
  });

  for (const route of routes) {
    test(`renders ${route}`, async ({ page }) => {
      await page.goto(`/#${route}`);
      await waitForAppReady(page);

      await expect
        .poll(async () => page.evaluate(() => window.location.hash))
        .toMatch(new RegExp(`^#${route.replace('/', '\\/')}`));
      await expect
        .poll(async () => {
          const text = await page.locator('#root').innerText();
          return text.trim().length;
        })
        .toBeGreaterThan(50);
    });
  }
});
