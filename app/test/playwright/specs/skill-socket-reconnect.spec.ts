import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Socket reconnect skill sync smoke', () => {
  test('reaches Home after login as baseline for post-reconnect flows', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-skill-socket-reconnect', '/home');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await expect(
      page.getByRole('button', { name: 'Ask your assistant anything...' })
    ).toBeVisible();
  });
});
