import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface PanelCheck {
  hash: string;
  markers: string[];
}

const panels: PanelCheck[] = [
  { hash: '/settings', markers: ['Settings', 'Appearance', 'Privacy'] },
  // The v1 memory data panel is gone; this slug redirects to the Memory
  // page's Documents chip (/connections?tab=brain&brain=documents).
  { hash: '/settings/memory-data', markers: ['Documents', 'Memory'] },
  { hash: '/settings/notifications-hub', markers: ['Plan & billing'] },
  { hash: '/settings/developer-options', markers: ['Developer', 'Debug', 'Advanced'] },
  { hash: '/settings/account', markers: ['Account', 'Plan & billing'] },
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
