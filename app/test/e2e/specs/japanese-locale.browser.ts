import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../../playwright/helpers/core-rpc';
import {
  browserElements,
  type BrowserPage,
  persistedBrowserLocale,
} from '../helpers/element-helpers';

async function settings(page: BrowserPage, user: string) {
  await bootAuthenticatedPage(page, user, '/settings/account');
  await dismissWalkthroughIfPresent(page);
}

async function localSettings(page: BrowserPage, user: string) {
  await bootAuthenticatedPage(page, user, '/settings/account');
  await callCoreRpc('openhuman.auth_store_session', {
    token: 'header.payload.local',
    userId: 'local',
    user: { _id: 'local', id: 'local', name: 'Local User', email: 'local@openhuman.local' },
  });
  // Installing a local credential intentionally returns onboarding to its
  // incomplete state. This test starts from an established local session.
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
  await page.reload();
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function expectJapanese(page: BrowserPage) {
  await expect(browserElements(page).language('言語')).toHaveValue('ja');
  await expect(browserElements(page).document).toHaveAttribute('lang', 'ja');
  await expect(browserElements(page).document).toHaveAttribute('dir', 'ltr');
  await expect(browserElements(page).chatTab).toContainText('チャット');
}

test.describe('Japanese UI locale', () => {
  test.describe.configure({ timeout: 180_000 });

  test('selects Japanese through Settings, restores it after reload, and switches back', async ({
    page,
  }) => {
    await settings(page, 'pw-japanese-switch');
    await browserElements(page).selectLanguage('Language', { label: '🇯🇵 日本語' });
    await expectJapanese(page);
    // Wait for redux-persist to write, rather than dispatching or seeding a locale.
    await expect.poll(() => persistedBrowserLocale(page)).toBe('ja');
    await page.reload();
    await expectJapanese(page);
    await browserElements(page).selectLanguage('言語', 'en');
    await expect(browserElements(page).language('Language')).toHaveValue('en');
    await expect(browserElements(page).document).toHaveAttribute('lang', 'en');
    await expect(browserElements(page).chatTab).toContainText('Chat');
    await expect.poll(() => persistedBrowserLocale(page)).toBe('en');
    await page.reload();
    await expect(browserElements(page).language('Language')).toHaveValue('en');
  });

  test('detects ja-JP on a fresh browser and preserves a manual English override', async ({
    browser,
    baseURL,
  }) => {
    const context = await browser.newContext({
      baseURL,
      locale: 'ja-JP',
      viewport: { width: 1280, height: 720 },
    });
    const page = await context.newPage();
    try {
      await settings(page, 'pw-japanese-detect');
      expect(await page.evaluate(() => navigator.language)).toBe('ja-JP');
      await expectJapanese(page);
      await expect(browserElements(page).language('言語')).toBeInViewport();
      await browserElements(page).selectLanguage('言語', 'en');
      await expect.poll(() => persistedBrowserLocale(page)).toBe('en');
      await page.reload();
      await expect(browserElements(page).language('Language')).toHaveValue('en');
      expect(await page.evaluate(() => navigator.language)).toBe('ja-JP');
    } finally {
      await context.close();
    }
  });

  test('renders Japanese memory counts and model attribution at the default viewport', async ({
    page,
  }) => {
    const fixtures: Record<string, unknown> = {
      'openhuman.memory_engine_get': {
        engine: 'tinycortex',
        has_key: true,
        status: 'ready',
        fetch_modes: [],
      },
      'openhuman.memory_engines_list': { engines: [] },
      'openhuman.memory_explore': {
        facet: 'kind',
        buckets: [{ value: 'document', count: 7 }],
        total: 7,
        missing: 0,
        more_buckets: 0,
        truncated: false,
      },
      'openhuman.memory_items_list': { items: [] },
      'openhuman.tokenjuice_savings_stats': {
        attributionModel: 'Qwen3.8-Flash-Next',
        total: {
          events: 3,
          originalTokens: 1000,
          compactedTokens: 500,
          tokensSaved: 500,
          costSavedUsd: 0,
        },
        byModel: {},
        byCompressor: {},
        cache: { entries: 0, bytes: 0 },
      },
    };
    // Only data-dependent read results are fixtures; authentication, locale
    // persistence and navigation still run through the shared Core harness.
    await page.route('**/rpc', async route => {
      const request = route.request().postDataJSON();
      if (request && Object.prototype.hasOwnProperty.call(fixtures, request.method)) {
        await route.fulfill({
          json: { jsonrpc: '2.0', id: request.id, result: fixtures[request.method] },
        });
      } else {
        await route.continue();
      }
    });
    await page.setViewportSize({ width: 1280, height: 720 });
    await settings(page, 'pw-japanese-interpolation');
    await browserElements(page).selectLanguage('Language', 'ja');
    await expect.poll(() => persistedBrowserLocale(page)).toBe('ja');
    await page.goto('/#/connections?tab=brain&brain=explorer');
    const count = browserElements(page).text('この場所の項目: 7件');
    await expect(count).toBeVisible();
    await count.scrollIntoViewIfNeeded();
    await expect(count).toBeInViewport();
    expect(await count.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
    await page.goto('/#/connections?tab=usage#tokens');
    const attribution = browserElements(page).text('Qwen3.8-Flash-Next のコストで計算');
    await expect(attribution).toBeVisible();
    await attribution.scrollIntoViewIfNeeded();
    await expect(attribution).toBeInViewport();
    expect(await attribution.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(
      true
    );
    await expect(browserElements(page).text('3 回の圧縮で')).toBeVisible();
  });

  test('renders the Japanese Composio direct-only explanation for a local session', async ({
    page,
  }) => {
    await localSettings(page, 'pw-japanese-composio-local');
    await browserElements(page).selectLanguage('Language', 'ja');
    await expect.poll(() => persistedBrowserLocale(page)).toBe('ja');
    await page.goto('/#/connections?tab=composio-key');
    await expect(
      browserElements(page).text(
        'この環境ではComposioのマネージド認証を利用できません。独自のComposio APIキーを入力するか、設定を後回しにしてください。'
      )
    ).toBeVisible();
  });
});
