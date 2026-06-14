import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Google Meet Connections tab', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-gmeet-connections-tab-user', '/connections?tab=meetings');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
  });

  test('opens the Meetings tab and shows the inline join form', async ({ page }) => {
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/connections');

    await expect(page.getByTestId('two-pane-nav-meetings')).toHaveAttribute('aria-current', 'page');

    // The join form renders inline on the Meetings tab (no banner/modal).
    await expect(page.getByText('Send OpenHuman to a meeting')).toBeVisible();
    await expect(page.getByText('Meeting link')).toBeVisible();
    await expect(page.locator('input[type="url"]')).toHaveCount(1);
    await expect(page.getByText('Zoom')).toHaveCount(0);
    await expect(page.getByText('Microsoft Teams')).toHaveCount(0);
  });
});
