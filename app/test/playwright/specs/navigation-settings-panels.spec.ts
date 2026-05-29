import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface PanelCheck {
  hash: string;
  markers: string[];
}

const panels: PanelCheck[] = [
  { hash: '/settings', markers: ['Settings', 'Appearance', 'Notifications'] },
  { hash: '/settings/memory-data', markers: ['Memory', 'Data', 'Storage'] },
  { hash: '/intelligence', markers: ['Memory', 'Intelligence'] },
  { hash: '/settings/developer-options', markers: ['Developer', 'Debug', 'Advanced'] },
  {
    hash: '/settings/billing',
    markers: ['Billing moved to the web', 'Open billing dashboard', 'credits'],
  },
  { hash: '/settings/appearance', markers: ['Appearance', 'Theme', 'Color'] },
  { hash: '/settings/tools', markers: ['Tools', 'Enable', 'Disable'] },
];

test.describe('Settings Panels', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-user');
  });

  for (const panel of panels) {
    test(`loads ${panel.hash}`, async ({ page }) => {
      await page.goto(`/#${panel.hash}`);
      await waitForAppReady(page);

      const text = await page.locator('#root').innerText();
      expect(text.trim().length).toBeGreaterThan(50);
      expect(panel.markers.some(marker => text.includes(marker))).toBe(true);
    });
  }
});
