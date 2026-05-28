import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Skill lifecycle smoke', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-skill-lifecycle-' + testSlug, '/skills');
  });

  test('skills page mounts and the skills_list RPC is reachable', async ({ page }) => {
    await waitForAppReady(page);
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/skills');

    const text = await page.locator('#root').innerText();
    expect(
      ['Composio Integrations', 'Install', 'Available', 'Channels'].some(marker =>
        text.includes(marker)
      )
    ).toBe(true);

    const rpcResult = await callCoreRpc<unknown>('openhuman.skills_list', {});
    const root = (rpcResult ?? {}) as Record<string, unknown>;
    const payload =
      root && typeof root === 'object' && 'result' in root
        ? (root.result as Record<string, unknown>)
        : root;
    expect(Array.isArray(payload.skills ?? [])).toBe(true);
  });
});
