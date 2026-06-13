// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-search';

async function clickSearchEngine(engine: string): Promise<void> {
  const clicked = await browser.execute(next => {
    const el = document.querySelector<HTMLButtonElement>(`[data-testid="search-engine-${next}"]`);
    if (!el) return false;
    el.click();
    return true;
  }, engine);
  expect(clicked).toBe(true);
}

async function selectedSearchEngine(): Promise<string | null> {
  return await browser.execute(() => {
    const selected = document.querySelector<HTMLElement>(
      '[data-testid^="search-engine-"][aria-checked="true"]'
    );
    return selected?.getAttribute('data-testid')?.replace('search-engine-', '') ?? null;
  });
}

async function getSearchSettings(): Promise<Record<string, unknown>> {
  const response = await callOpenhumanRpc('openhuman.config_get_search_settings', {});
  expect(response.ok).toBe(true);
  return response.result?.result ?? {};
}

describe('Settings - Search', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('persists Disabled search engine from the search settings panel', async () => {
    const reset = await callOpenhumanRpc('openhuman.config_update_search_settings', {
      engine: 'managed',
    });
    expect(reset.ok).toBe(true);

    await navigateViaHash('/settings/search');
    await browser.waitUntil(
      async () =>
        Boolean(
          await browser.execute(() =>
            document.querySelector('[data-testid="search-settings-panel"]')
          )
        ),
      { timeout: 15_000, interval: 250, timeoutMsg: 'search settings panel did not render' }
    );

    expect(await selectedSearchEngine()).toBe('managed');

    await clickSearchEngine('disabled');

    await browser.waitUntil(async () => (await selectedSearchEngine()) === 'disabled', {
      timeout: 10_000,
      interval: 250,
      timeoutMsg: 'disabled search engine did not become selected',
    });

    await browser.waitUntil(
      async () => {
        const settings = await getSearchSettings();
        return settings.engine === 'disabled' && settings.effective_engine === 'disabled';
      },
      {
        timeout: 10_000,
        interval: 500,
        timeoutMsg: 'disabled search engine was not persisted to core config',
      }
    );
  });
});
