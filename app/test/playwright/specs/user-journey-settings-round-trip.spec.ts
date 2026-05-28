import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

const PANEL_TIMEOUT = 10_000;

interface PanelCheck {
  hash: string;
  markers: string[];
}

const panels: PanelCheck[] = [
  { hash: '/settings', markers: ['Settings', 'Appearance', 'Notifications'] },
  { hash: '/settings/memory-data', markers: ['Memory', 'Data', 'Storage'] },
  { hash: '/settings/developer-options', markers: ['Developer', 'Debug', 'Advanced'] },
  {
    hash: '/settings/billing',
    markers: ['Billing moved to the web', 'Open billing dashboard', 'credits'],
  },
  { hash: '/home', markers: ['Ask your assistant anything', 'Your device is connected'] },
  { hash: '/chat', markers: ['Threads', 'New thread', 'Chat'] },
];

async function waitForPanelLoad(page: Parameters<typeof test>[0]['page']) {
  await waitForAppReady(page);
  const chars = await page.locator('#root').innerText();
  expect(chars.trim().length).toBeGreaterThan(50);
}

test.describe('User journey - settings round-trip', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-settings-round-trip-' + testSlug, '/home');
  });

  test('starts on /home after login', async ({ page }) => {
    await waitForAppReady(page);
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: PANEL_TIMEOUT })
      .toMatch(/^#\/home/);
    const text = await page.locator('#root').innerText();
    expect(
      ['Ask your assistant anything', 'Your device is connected'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);
  });

  for (const panel of panels) {
    test(`${panel.hash} loads with non-trivial content`, async ({ page }) => {
      await page.goto(`/#${panel.hash}`);
      await waitForPanelLoad(page);

      const text = await page.locator('#root').innerText();
      expect(panel.markers.some(marker => text.includes(marker))).toBe(true);
    });
  }
});
