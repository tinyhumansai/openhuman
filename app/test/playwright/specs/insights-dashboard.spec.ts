import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Insights Dashboard', () => {
  test('renders the memory workspace and actions toolbar', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-insights-user', '/intelligence');
    await waitForAppReady(page);

    // The Intelligence page now defaults to the "Agent Tasks" tab (#2998), so
    // select the Memory tab before asserting its workspace renders.
    await page.getByRole('tab', { name: 'Memory', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Memory', exact: true })).toBeVisible();
    await expect(page.locator('[data-testid="memory-workspace"]')).toBeVisible();
    await expect(page.locator('[data-testid="memory-actions"]')).toBeVisible();
  });
});
