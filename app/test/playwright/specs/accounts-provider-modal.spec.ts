import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
  waitForAppReady,
} from '../helpers/core-rpc';

const BASE_PICKER_PROVIDERS = [
  { id: 'whatsapp', label: 'WhatsApp Web' },
  { id: 'wechat', label: 'WeChat Web' },
  { id: 'telegram', label: 'Telegram Web' },
  { id: 'linkedin', label: 'LinkedIn' },
  { id: 'slack', label: 'Slack' },
  { id: 'discord', label: 'Discord' },
] as const;

const HIDDEN_PROVIDER_IDS = ['google-meet', 'zoom'] as const;
const DEV_PICKER_PROVIDER = { id: 'browserscan', label: 'BrowserScan (dev)' } as const;

async function openAddAccountModal(page: import('@playwright/test').Page) {
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

async function visibleProviderIds(page: import('@playwright/test').Page): Promise<string[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('[data-testid^="add-account-provider-"]'))
      .map(node => node.getAttribute('data-testid')?.replace('add-account-provider-', ''))
      .filter((value): value is string => Boolean(value))
      .sort()
  );
}

async function registeredProviders(page: import('@playwright/test').Page): Promise<string[]> {
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

async function bootAccountsPage(page: import('@playwright/test').Page, userId: string) {
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

test.describe('Accounts Provider Modal', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAccountsPage(page, `pw-accounts-provider-modal-${slug}`);
  });

  test('shows exposed providers and keeps hidden providers out of the picker', async ({ page }) => {
    await openAddAccountModal(page);

    for (const provider of BASE_PICKER_PROVIDERS) {
      await expect(page.getByTestId(`add-account-provider-${provider.id}`)).toContainText(
        provider.label
      );
    }

    for (const providerId of HIDDEN_PROVIDER_IDS) {
      await expect(page.getByTestId(`add-account-provider-${providerId}`)).toHaveCount(0);
    }

    const ids = await visibleProviderIds(page);
    for (const provider of BASE_PICKER_PROVIDERS) {
      expect(ids).toContain(provider.id);
    }
    expect(ids).not.toContain('google-meet');
    expect(ids).not.toContain('zoom');

    await page.keyboard.press('Escape');
    await expect(page.getByTestId('add-account-modal')).toHaveCount(0);
  });

  test('registers each visible provider through the picker interaction', async ({ page }) => {
    await openAddAccountModal(page);
    const initiallyVisibleIds = await visibleProviderIds(page);
    const providersToRegister = BASE_PICKER_PROVIDERS.filter(provider =>
      initiallyVisibleIds.includes(provider.id)
    );
    if (initiallyVisibleIds.includes(DEV_PICKER_PROVIDER.id)) {
      providersToRegister.push(DEV_PICKER_PROVIDER);
    }
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('add-account-modal')).toHaveCount(0);

    for (const provider of providersToRegister) {
      await page.goto('/#/chat');
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);

      await openAddAccountModal(page);
      await page.getByTestId(`add-account-provider-${provider.id}`).click();
      await expect(page.getByTestId('add-account-modal')).toHaveCount(0);

      await expect
        .poll(async () => registeredProviders(page), {
          message: `Redux accounts slice never recorded provider ${provider.id}`,
        })
        .toContain(provider.id);
    }

    const providers = await registeredProviders(page);
    for (const provider of providersToRegister) {
      expect(providers).toContain(provider.id);
    }
    expect(providers).not.toContain('google-meet');
    expect(providers).not.toContain('zoom');
  });
});
