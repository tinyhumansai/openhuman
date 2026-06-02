import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Insights Dashboard', () => {
  test('renders the memory workspace and actions toolbar', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-insights-user', '/intelligence');
    await waitForAppReady(page);
    // /intelligence boots on the Tasks tab (default in Intelligence.tsx since
    // #2998). The walkthrough portal can sit on top of the pill bar and the
    // memory-workspace testid only mounts when tab=memory, so we dismiss the
    // overlay and click the Memory pill before asserting the panel chrome.
    await dismissWalkthroughIfPresent(page);
    await page.getByRole('tab', { name: 'Memory', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Memory', exact: true })).toBeVisible({
      timeout: 15_000,
    });
    await expect(page.locator('[data-testid="memory-workspace"]')).toBeVisible();
    await expect(page.locator('[data-testid="memory-actions"]')).toBeVisible();
  });
});
