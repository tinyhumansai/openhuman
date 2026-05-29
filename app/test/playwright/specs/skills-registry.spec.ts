import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
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
    window.location.hash = '/skills';
  });
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
    .toContain('/skills');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Skills registry flow', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await openSkillsPage(page, 'pw-skills-registry-' + testSlug);
  });

  test('navigates to /skills and renders the current tabs', async ({ page }) => {
    await expect(page.getByRole('tab', { name: 'Composio' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Channels' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'MCP Servers' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Composio Integrations' })).toBeVisible();
  });

  test('shows at least one known Composio integration name', async ({ page }) => {
    await expect(
      page.getByText(/Gmail|Notion|Telegram|GitHub|Google Drive/, { exact: false }).first()
    ).toBeVisible();
  });

  test('channels tab renders messaging connectors', async ({ page }) => {
    await page.getByRole('tab', { name: 'Channels' }).click();
    await expect(page.getByRole('heading', { name: 'Channels' })).toBeVisible();
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });

  test('mcp tab shows the placeholder panel', async ({ page }) => {
    await page.getByRole('tab', { name: 'MCP Servers' }).click();
    await expect(page.getByRole('heading', { name: 'MCP Servers' }).first()).toBeVisible();
    await expect(page.getByText(/coming soon|early alpha|MCP/i).first()).toBeVisible();
  });
});
