import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Settings - AI & Skills', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-ai-user');
  });

  test('mounts LLM panel and shows provider/routing controls', async ({ page }) => {
    // /settings/llm now redirects to the Connections page (LLM moved there);
    // the AIPanel renders on the Connections LLM tab.
    await page.goto('/#/settings/llm');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections');
    await expect(page.getByRole('heading', { name: 'LLM Providers', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Routing', exact: true })).toBeVisible();
  });

  test('mounts Tools panel and shows tool toggles', async ({ page }) => {
    await page.goto('/#/settings/tools');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    // The two-pane sidebar also renders a "Tools" nav label, so scope to first.
    await expect(page.getByText('Tools').first()).toBeVisible();
    await expect(page.getByText(/Filesystem|Shell/).first()).toBeVisible();
  });
});
