import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Insights Dashboard', () => {
  test('renders the memory workspace and actions toolbar', async ({ page }) => {
    // Phase 3: Memory moved from /activity (no Memory tab there anymore) to
    // /settings/intelligence which renders the full Intelligence page including
    // the Memory tab. The Memory tab is NOT dev-only in Intelligence.tsx (only
    // "council" is gated), so no developer mode seeding is needed.
    await bootAuthenticatedPage(page, 'pw-insights-user', '/settings/intelligence');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    // /settings/intelligence defaults to the Tasks tab — click Memory pill.
    await page.getByRole('tab', { name: 'Memory', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Memory', exact: true })).toBeVisible({
      timeout: 15_000,
    });
    await expect(page.locator('[data-testid="memory-workspace"]')).toBeVisible();
    await expect(page.locator('[data-testid="memory-actions"]')).toBeVisible();
  });
});
