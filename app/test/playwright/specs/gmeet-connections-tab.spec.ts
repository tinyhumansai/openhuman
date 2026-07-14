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

  test('opens the Meetings tab and shows the multi-platform composer', async ({ page }) => {
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/connections');

    await expect(page.getByTestId('two-pane-nav-meetings')).toHaveAttribute('aria-current', 'page');

    // The redesigned composer renders inline on the Meetings tab (no banner/modal).
    await expect(page.getByText('Send OpenHuman to a meeting')).toBeVisible();
    await expect(page.locator('input[type="url"]')).toHaveCount(1);

    // The redesign replaced the single-platform form with a platform selector;
    // all four platforms are now selectable radio chips.
    await expect(page.getByRole('radio', { name: /google meet/i })).toBeVisible();
    await expect(page.getByRole('radio', { name: /zoom/i })).toBeVisible();
    await expect(page.getByRole('radio', { name: /microsoft teams/i })).toBeVisible();
    await expect(page.getByRole('radio', { name: /webex/i })).toBeVisible();
  });
});
