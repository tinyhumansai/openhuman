import { expect, type Page, test } from '@playwright/test';

import { mockRequests, setMockBehavior } from '../helpers/chat-drive';
import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const CATALOG_SIZE = 32;
const PAGE_SIZE = 25;

async function openRegistry(page: Page, userId: string) {
  await page.route('**/rpc', async (route, request) => {
    try {
      const body = JSON.parse(request.postData() || '{}');
      if (body.method === 'openhuman.composio_list_agent_ready_toolkits') {
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: { result: { toolkits: [] }, logs: [] },
          }),
        });
        return;
      }
    } catch {
      /* pass through */
    }
    await route.continue();
  });
  await bootAuthenticatedPage(page, userId, '/connections?tab=skills');
  await expect
    .poll(() => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toContain('tab=skills');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await page.getByTestId('skill-explorer-tab-registry').click();
  await expect(page.getByTestId('skill-search-input')).toBeVisible({ timeout: 20_000 });
}

const catalogRows = (page: Page) => page.locator('[data-testid^="registry-tile-"]');

const SCAN_BLOCKED_SKILL = 'fixture-skill-07';

async function documentFetches(name: string): Promise<number> {
  return (await mockRequests()).filter(entry => entry.url.includes(`/skills/${name}/SKILL.md`))
    .length;
}

test.describe('Skill registry catalog over the tinyskills registry', () => {
  test('pages, filters and reports freshness over JSON-RPC', async () => {
    test.setTimeout(60_000);
    const first = await callCoreRpc<{
      entries: Array<{ id: string; installable: boolean; registry: string }>;
      total: number;
      page: number;
      total_pages: number;
      freshness: string;
      last_error: unknown;
    }>('openhuman.skill_registry_browse', { page: 1, page_size: PAGE_SIZE, force_refresh: true });
    expect(first.total).toBe(CATALOG_SIZE);
    expect(first.entries).toHaveLength(PAGE_SIZE);
    expect(first.total_pages).toBe(2);
    expect(first.freshness).toBe('live');
    expect(first.last_error).toBeNull();
    expect(first.entries.every(entry => entry.installable && entry.registry === 'hermes')).toBe(
      true
    );

    const second = await callCoreRpc<{ entries: unknown[]; page: number }>(
      'openhuman.skill_registry_browse',
      { page: 2, page_size: PAGE_SIZE }
    );
    expect(second.page).toBe(2);
    expect(second.entries).toHaveLength(CATALOG_SIZE - PAGE_SIZE);

    const pack = await callCoreRpc<{ total: number }>('openhuman.skill_registry_browse', {
      page: 1,
      page_size: PAGE_SIZE,
      sources: ['fixture-pack'],
    });
    expect(pack.total).toBe(30);

    const detail = await callCoreRpc<{ id: string; overview: string; download_url: string }>(
      'openhuman.skill_registry_detail',
      { entry_id: 'docker-management' }
    );
    expect(detail.id).toBe('docker-management');
    expect(detail.download_url).toContain('/skills/docker-management/SKILL.md');
  });

  test('browses page by page, installs a skill and uninstalls it', async ({ page }) => {
    test.setTimeout(90_000);
    await openRegistry(page, 'pw-skills-catalog-lifecycle');

    await expect(catalogRows(page)).toHaveCount(PAGE_SIZE, { timeout: 30_000 });
    const pager = page.getByTestId('registry-pagination');
    await pager.getByRole('button', { name: 'Next page' }).click();
    await expect(catalogRows(page)).toHaveCount(CATALOG_SIZE - PAGE_SIZE, { timeout: 15_000 });

    await page.getByTestId('skill-search-input').fill('docker');
    const install = page.getByTestId('registry-install-docker-management');
    await expect(install).toBeVisible({ timeout: 15_000 });
    await install.click();
    await expect(
      page.getByTestId('registry-tile-docker-management').getByText('Installed', { exact: true })
    ).toBeVisible({ timeout: 30_000 });

    await page.getByTestId('skill-explorer-tab-installed').click();
    const uninstall = page.getByTestId('skill-uninstall-docker-management');
    await expect(uninstall).toBeVisible({ timeout: 15_000 });
    await uninstall.click();
    await page.getByTestId('uninstall-skill-confirm').click();
    await expect(uninstall).toHaveCount(0, { timeout: 15_000 });
  });

  test('prompts on a scan-blocked skill: Block keeps it out, Install anyway installs only the reviewed document', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await setMockBehavior('skillRegistryScanBlocked', SCAN_BLOCKED_SKILL);
    try {
      await openRegistry(page, 'pw-skills-catalog-scan-blocked');
      await page.getByTestId('skill-search-input').fill(SCAN_BLOCKED_SKILL);
      const install = page.getByTestId(`registry-install-${SCAN_BLOCKED_SKILL}`);
      await expect(install).toBeVisible({ timeout: 15_000 });

      const before = await documentFetches(SCAN_BLOCKED_SKILL);
      await install.click();
      const dialog = page.getByTestId('scan-blocked-dialog');
      await expect(dialog).toBeVisible({ timeout: 30_000 });
      await expect(dialog.getByTestId('scan-blocked-findings')).toContainText('U+200B');
      await expect(page.getByTestId('scan-blocked-block')).toBeFocused();
      expect(await documentFetches(SCAN_BLOCKED_SKILL)).toBe(before + 2);

      await page.getByTestId('scan-blocked-block').click();
      await expect(dialog).toHaveCount(0);
      await expect(install).toBeVisible();
      await expect(
        page
          .getByTestId(`registry-tile-${SCAN_BLOCKED_SKILL}`)
          .getByText('Installed', { exact: true })
      ).toHaveCount(0);

      await install.click();
      await expect(dialog).toBeVisible({ timeout: 30_000 });
      await setMockBehavior('skillRegistryScanVariant', 'changed');
      const beforeChanged = await documentFetches(SCAN_BLOCKED_SKILL);
      await page.getByTestId('scan-blocked-install-anyway').click();
      await expect
        .poll(() => documentFetches(SCAN_BLOCKED_SKILL), { timeout: 30_000 })
        .toBe(beforeChanged + 2);
      await expect(page.getByTestId('scan-blocked-install-anyway')).toBeEnabled({
        timeout: 30_000,
      });
      await expect(dialog).toBeVisible();
      await expect(
        page
          .getByTestId(`registry-tile-${SCAN_BLOCKED_SKILL}`)
          .getByText('Installed', { exact: true })
      ).toHaveCount(0);

      await page.getByTestId('scan-blocked-install-anyway').click();
      await expect(dialog).toHaveCount(0, { timeout: 30_000 });
      await expect(
        page
          .getByTestId(`registry-tile-${SCAN_BLOCKED_SKILL}`)
          .getByText('Installed', { exact: true })
      ).toBeVisible({ timeout: 30_000 });
    } finally {
      await setMockBehavior('skillRegistryScanBlocked', '');
      await setMockBehavior('skillRegistryScanVariant', '');
      await callCoreRpc('openhuman.skill_registry_uninstall', { name: SCAN_BLOCKED_SKILL }).catch(
        () => undefined
      );
    }
  });

  test('keeps showing the saved catalog offline and waits out the 45s refresh cooldown', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await openRegistry(page, 'pw-skills-catalog-offline');
    await expect(catalogRows(page)).toHaveCount(PAGE_SIZE, { timeout: 30_000 });

    try {
      await setMockBehavior('skillRegistryUnavailable', 'true');
      await page.getByRole('button', { name: 'Refresh registry' }).click();

      await expect(page.getByTestId('registry-offline')).toBeVisible({ timeout: 30_000 });
      await expect(catalogRows(page)).toHaveCount(PAGE_SIZE);

      const catalogFetches = async () =>
        (await mockRequests()).filter(entry => entry.url.includes('/skills/catalog.json')).length;
      const beforeRetry = await catalogFetches();
      await page.getByTestId('registry-retry').click();
      await expect(page.getByTestId('registry-offline')).toBeVisible({ timeout: 30_000 });
      await expect(catalogRows(page)).toHaveCount(PAGE_SIZE);
      expect(await catalogFetches()).toBe(beforeRetry);
    } finally {
      await setMockBehavior('skillRegistryUnavailable', 'false');
    }
  });
});
