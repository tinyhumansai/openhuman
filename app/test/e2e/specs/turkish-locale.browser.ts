import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
} from '../../playwright/helpers/core-rpc';
import { browserElements, persistedBrowserLocale } from '../helpers/element-helpers';

test.describe('Turkish UI locale', () => {
  test.describe.configure({ timeout: 180_000 });

  test('selects Turkish through Settings, restores it after reload, and switches back', async ({
    page,
  }) => {
    await bootAuthenticatedPage(page, 'pw-turkish-switch', '/settings/account');
    await dismissWalkthroughIfPresent(page);

    await browserElements(page).selectLanguage('Language', { label: '🇹🇷 Türkçe' });
    await expect(browserElements(page).language('Dil')).toHaveValue('tr');
    await expect(browserElements(page).document).toHaveAttribute('lang', 'tr');
    await expect(browserElements(page).document).toHaveAttribute('dir', 'ltr');
    await expect(browserElements(page).chatTab).toContainText('Sohbet');
    await expect.poll(() => persistedBrowserLocale(page)).toBe('tr');

    await page.reload();
    await expect(browserElements(page).language('Dil')).toHaveValue('tr');
    await expect(browserElements(page).chatTab).toContainText('Sohbet');

    await browserElements(page).selectLanguage('Dil', 'en');
    await expect(browserElements(page).language('Language')).toHaveValue('en');
    await expect(browserElements(page).document).toHaveAttribute('lang', 'en');
    await expect(browserElements(page).chatTab).toContainText('Chat');
    await expect.poll(() => persistedBrowserLocale(page)).toBe('en');

    await page.reload();
    await expect(browserElements(page).language('Language')).toHaveValue('en');
  });
});
