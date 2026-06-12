import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

test.describe('Skill lifecycle smoke', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    // Phase 2: /skills redirected to /connections
    await bootAuthenticatedPage(page, 'pw-skill-lifecycle-' + testSlug, '/connections');
  });

  test('connections page mounts and the workflows_list RPC is reachable', async ({ page }) => {
    await waitForAppReady(page);
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/connections');

    const text = await page.locator('#root').innerText();
    // Connections page tab labels (IA revamp): Apps/Messaging/Tools/Explorer/Talents.
    expect(
      ['Apps', 'Messaging', 'Tools', 'Explorer', 'Talents'].some(marker => text.includes(marker))
    ).toBe(true);

    const rpcResult = await callCoreRpc<unknown>('openhuman.workflows_list', {});
    const root = (rpcResult ?? {}) as Record<string, unknown>;
    const payload =
      root && typeof root === 'object' && 'result' in root
        ? (root.result as Record<string, unknown>)
        : root;
    expect(Array.isArray(payload.skills ?? [])).toBe(true);
  });
});
