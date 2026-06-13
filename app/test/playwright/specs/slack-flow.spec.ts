import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
  waitForAppReady,
} from '../helpers/core-rpc';

async function openAddAccountModal(page: Page) {
  const modal = page.getByTestId('add-account-modal');
  await page.getByTestId('accounts-add-button').click({ force: true });
  try {
    await expect(modal).toBeVisible({ timeout: 3_000 });
    return;
  } catch {
    await dismissWalkthroughIfPresent(page);
    await page.evaluate(() => {
      const button = document.querySelector('[data-testid="accounts-add-button"]');
      if (button instanceof HTMLElement) button.click();
    });
  }
  await expect(modal).toBeVisible();
}

async function registeredProviders(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState: () => { accounts?: { accounts?: Record<string, { provider?: string }> } };
        };
      }
    ).__OPENHUMAN_STORE__;
    const accounts = store?.getState()?.accounts?.accounts ?? {};
    return Object.values(accounts)
      .map(account => account.provider)
      .filter((provider): provider is string => Boolean(provider))
      .sort();
  });
}

async function bootAccountsPage(page: Page, userId: string) {
  await bootRuntimeReadyGuestPage(page);
  try {
    await signInViaBypassUser(page, userId);
  } catch {
    await bootRuntimeReadyGuestPage(page);
    await signInViaBypassUser(page, userId);
  }
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('accounts-page')).toBeVisible();
}

test.describe('Slack account integration smoke', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAccountsPage(page, `pw-slack-flow-${slug}`);
  });

  test('shows Slack as an addable provider in the Add Account modal', async ({ page }) => {
    await openAddAccountModal(page);
    await expect(page.getByTestId('add-account-provider-slack')).toContainText('Slack');
  });

  test('selecting Slack closes the modal and registers an account on the rail', async ({
    page,
  }) => {
    await openAddAccountModal(page);
    await page.getByTestId('add-account-provider-slack').click();
    await expect(page.getByTestId('add-account-modal')).toHaveCount(0);

    await expect
      .poll(async () => registeredProviders(page), {
        message:
          'Redux accounts slice never recorded a slack provider after picking the Slack tile',
      })
      .toContain('slack');
  });
});
