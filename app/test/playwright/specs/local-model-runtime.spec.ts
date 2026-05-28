import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Local model runtime flow', () => {
  test('shows direct-runtime guidance instead of app-managed bootstrap controls', async ({
    page,
  }) => {
    await bootAuthenticatedPage(page, 'pw-local-model-runtime', '/settings/local-model-debug');
    await waitForAppReady(page);

    const text = await page.locator('#root').innerText();
    expect(
      [
        'Ollama runtime unavailable',
        'Manage the Ollama process and model pulls outside OpenHuman',
        'Ollama docs',
        'Local model runtime',
      ].some(marker => text.includes(marker))
    ).toBe(true);
  });
});
