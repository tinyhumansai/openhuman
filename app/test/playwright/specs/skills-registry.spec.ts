import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

async function openSkillsPage(page: Parameters<typeof test>[0]['page'], userId: string) {
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    // /skills redirects to /connections (Phase 2 rename)
    window.location.hash = '/connections';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
    .toContain('/connections');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Skills registry flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await openSkillsPage(page, 'pw-skills-registry-' + testSlug);
  });

  test('navigates to /connections and renders the current tabs', async ({ page }) => {
    // Phase 2: tabs renamed — Apps (was Composio), Messaging (was Channels), Tools (was MCP).
    // `exact: true` on Tools avoids colliding with the "Tools & Automation" skill
    // category pill (also role="tab").
    await expect(page.getByRole('tab', { name: 'Apps', exact: true })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Messaging', exact: true })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Tools', exact: true })).toBeVisible();
    await page.getByRole('tab', { name: 'Apps', exact: true }).click();
    // Heading reads "Composio Integrations" (skills.integrations); the tab is "Apps"
    await expect(
      page.getByRole('heading', { name: 'Composio Integrations', exact: true })
    ).toBeVisible();
    await expect(
      page.getByText(/Gmail|Notion|Telegram|GitHub|Google Drive/, { exact: false }).first()
    ).toBeVisible();
  });

  test('shows at least one known Composio integration name', async ({ page }) => {
    await expect(
      page.getByText(/Gmail|Notion|Telegram|GitHub|Google Drive/, { exact: false }).first()
    ).toBeVisible();
  });

  test('messaging tab renders messaging connectors', async ({ page }) => {
    // Phase 2: "Channels" tab renamed to "Messaging". The standalone "Channels"
    // heading is gone; the connector cards are the meaningful content.
    await page.getByRole('tab', { name: 'Messaging', exact: true }).click();
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });

  test('tools tab renders the server table', async ({ page }) => {
    // Phase 2: "MCP Servers" tab renamed to "Tools" (exact to avoid the
    // "Tools & Automation" category pill).
    await page.getByRole('tab', { name: 'Tools', exact: true }).click();
    await expect(
      page
        .getByRole('searchbox')
        .or(page.getByPlaceholder(/search/i))
        .first()
    ).toBeVisible();
    await expect(page.getByText(/^All$|^Installed$|^Registry$/i).first()).toBeVisible();
  });
});

test.describe('Skill registry RPC smoke', () => {
  test('sources returns upstream source names', async () => {
    const result = await callCoreRpc<{ sources: string[] }>('openhuman.skill_registry_sources');
    expect(result.sources).toBeDefined();
    expect(result.sources.length).toBeGreaterThan(0);

    for (const source of result.sources) {
      expect(typeof source).toBe('string');
      expect(source.length).toBeGreaterThan(0);
    }
  });

  test('browse returns catalog entries', async () => {
    test.setTimeout(30_000);
    const result = await callCoreRpc<{
      entries: Array<{
        id: string;
        name: string;
        source: string;
        category: string;
        download_url: string;
      }>;
    }>('openhuman.skill_registry_browse', { force_refresh: true });
    expect(result.entries).toBeDefined();
    expect(result.entries.length).toBeGreaterThan(0);

    for (const entry of result.entries.slice(0, 5)) {
      expect(entry.id).toBeTruthy();
      expect(entry.name).toBeTruthy();
      expect(entry.source).toBeTruthy();
    }
  });

  test('search filters entries by query', async () => {
    test.setTimeout(30_000);
    const result = await callCoreRpc<{
      entries: Array<{ id: string; name: string; description: string }>;
    }>('openhuman.skill_registry_search', { query: 'git' });
    expect(result.entries).toBeDefined();
    expect(result.entries.length).toBeGreaterThan(0);
  });

  test('search for docker returns results', async () => {
    test.setTimeout(30_000);
    const result = await callCoreRpc<{
      entries: Array<{ id: string; name: string; description: string; source: string }>;
    }>('openhuman.skill_registry_search', { query: 'docker' });
    expect(result.entries).toBeDefined();
    expect(result.entries.length).toBeGreaterThan(0);

    const hasDockerMatch = result.entries.some(
      e => e.name.toLowerCase().includes('docker') || e.description.toLowerCase().includes('docker')
    );
    expect(hasDockerMatch).toBe(true);
  });

  test('search with source filter narrows results', async () => {
    test.setTimeout(30_000);
    const all = await callCoreRpc<{ entries: Array<{ id: string; source: string }> }>(
      'openhuman.skill_registry_search',
      { query: 'git' }
    );

    const filtered = await callCoreRpc<{ entries: Array<{ id: string; source: string }> }>(
      'openhuman.skill_registry_search',
      { query: 'git', source: 'built-in' }
    );

    expect(filtered.entries.length).toBeLessThanOrEqual(all.entries.length);
    for (const entry of filtered.entries) {
      expect(entry.source).toBe('built-in');
    }
  });

  test('search with empty query returns all entries', async () => {
    test.setTimeout(30_000);
    const all = await callCoreRpc<{ entries: Array<unknown> }>('openhuman.skill_registry_search', {
      query: '',
    });
    expect(all.entries.length).toBeGreaterThan(0);
  });
});
