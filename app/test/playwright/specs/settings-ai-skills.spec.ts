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
    await page.goto('/#/settings/llm');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByRole('button', { name: 'AI', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'LLM Providers', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Routing', exact: true })).toBeVisible();
  });

  test('mounts Tools panel and shows tool toggles', async ({ page }) => {
    await page.goto('/#/settings/tools');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Tools')).toBeVisible();
    await expect(page.getByText(/Filesystem|Shell/).first()).toBeVisible();
  });
});
