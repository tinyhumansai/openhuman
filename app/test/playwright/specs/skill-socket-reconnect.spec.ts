import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Socket reconnect skill sync smoke', () => {
  test('reaches the chat surface after login as baseline for post-reconnect flows', async ({
    page,
  }) => {
    // Home folded into the unified chat surface: post-login landing is /chat,
    // whose "new window" empty state renders the former Home hero card.
    await bootAuthenticatedPage(page, 'pw-skill-socket-reconnect', '/home');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    await expect(page.locator('[data-walkthrough="home-card"]')).toBeVisible();
  });
});
