import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Local model runtime flow', () => {
  test('shows direct-runtime guidance instead of app-managed bootstrap controls', async ({
    page,
  }) => {
    await bootAuthenticatedPage(page, 'pw-local-model-runtime', '/settings/local-model-debug');
    await waitForAppReady(page);

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections');
  });
});
