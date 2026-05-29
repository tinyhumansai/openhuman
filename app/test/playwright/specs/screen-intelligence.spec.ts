import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Screen Intelligence', () => {
  test('opens the Screen Intelligence settings route', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-screen-intelligence', '/settings/screen-intelligence');
    await waitForAppReady(page);

    const text = await page.locator('#root').innerText();
    expect(text.includes('Screen Awareness')).toBe(true);
  });

  test('debug route reaches a stable success or unsupported/failure state', async ({ page }) => {
    await bootAuthenticatedPage(
      page,
      'pw-screen-intelligence-debug',
      '/settings/screen-awareness-debug'
    );
    await waitForAppReady(page);

    const text = await page.locator('#root').innerText();
    expect(
      [
        'Screen Awareness',
        'screen capture is unsupported on this platform',
        'screen recording permission is not granted',
        'Capture test',
        'Test capture',
      ].some(marker => text.includes(marker))
    ).toBe(true);
  });
});
