import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

async function bootReviewedFlow(page: import('@playwright/test').Page, userId: string) {
  await bootRuntimeReadyGuestPage(page);
  try {
    await signInViaCallbackToken(page, userId);
  } catch {
    await bootRuntimeReadyGuestPage(page);
    await signInViaCallbackToken(page, userId);
  }
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Agent review - canonical onboarding + privacy flow', () => {
  test('launches, reaches the shell, and opens the privacy panel', async ({ page }) => {
    await bootReviewedFlow(page, 'pw-agent-review');

    // The composer is the durable readiness marker for the unified chat shell;
    // sidebar copy and empty-state headings change independently of readiness.
    await expect(page.getByTestId('chat-message-input')).toBeVisible();

    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);

    await expect(page.getByTestId('settings-privacy-panel')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Product Analytics' })).toBeVisible();
    await expect(page.getByText('Share Product Analytics and Diagnostics')).toBeVisible();
  });
});
