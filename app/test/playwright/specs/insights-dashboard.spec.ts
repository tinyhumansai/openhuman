import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Insights Dashboard', () => {
  test('renders the memory workspace and actions toolbar', async ({ page }) => {
    // Memory's dashboard is the first-class Brain graph surface now.
    await bootAuthenticatedPage(page, 'pw-insights-user', '/brain?tab=graph');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await expect(page.getByText('Graph', { exact: true }).first()).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('[data-testid="memory-actions"]')).toBeVisible();
    await expect(
      page.locator('[data-testid="memory-graph-svg"], [data-testid="memory-graph-empty"]')
    ).toBeVisible();
  });
});
