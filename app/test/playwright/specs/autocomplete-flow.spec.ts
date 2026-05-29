import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Autocomplete Flow', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-autocomplete-flow-user');
  });

  test('mounts the autocomplete settings panel and renders runtime status', async ({ page }) => {
    await page.goto('/#/settings/autocomplete');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText('Autocomplete')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
    await expect(page.getByText('Runtime')).toBeVisible();
    await expect(page.getByText(/Running:\s+(Yes|No)/)).toBeVisible();
    await expect(page.getByText(/Enabled:\s+(Yes|No)/)).toBeVisible();
  });

  test('renders the runtime action controls and advanced-settings CTA', async ({ page }) => {
    await page.goto('/#/settings/autocomplete');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByRole('button', { name: 'Start' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Advanced settings' })).toBeVisible();
  });
});
