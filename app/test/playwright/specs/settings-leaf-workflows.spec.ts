import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function openSettings(page: Page, userId: string, hash: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, hash);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function themeState(
  page: Page
): Promise<{ mode?: string; tabBarLabels?: string; agentMessageViewMode?: string }> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => {
            theme?: { mode?: string; tabBarLabels?: string; agentMessageViewMode?: string };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store?.getState?.().theme ?? {};
  });
}

async function persistedThemeState(
  page: Page
): Promise<{ mode?: string; tabBarLabels?: string; agentMessageViewMode?: string }> {
  return page.evaluate(() => {
    const raw = localStorage.getItem('persist:theme');
    if (!raw) return {};
    try {
      const parsed = JSON.parse(raw) as Record<string, string>;
      return {
        mode: parsed.mode ? JSON.parse(parsed.mode) : undefined,
        tabBarLabels: parsed.tabBarLabels ? JSON.parse(parsed.tabBarLabels) : undefined,
        agentMessageViewMode: parsed.agentMessageViewMode
          ? JSON.parse(parsed.agentMessageViewMode)
          : undefined,
      };
    } catch {
      return {};
    }
  });
}

function unwrap<T>(value: T | { result: T }): T {
  if (value && typeof value === 'object' && 'result' in value) {
    return (value as { result: T }).result;
  }
  return value as T;
}

test.describe('Settings leaf workflows', () => {
  test('appearance theme persists in app state', async ({ page }) => {
    await openSettings(page, 'pw-settings-appearance', '/settings/appearance');

    // Panel title dropped in the PanelPage migration; the theme radios confirm
    // the Appearance panel mounted.
    await expect(page.getByRole('radio', { name: /Dark/ })).toBeVisible();
    await page.getByRole('radio', { name: /Dark/ }).click();
    await expect.poll(() => themeState(page)).toMatchObject({ mode: 'dark' });
    await expect.poll(() => persistedThemeState(page)).toMatchObject({ mode: 'dark' });

    await page.reload();
    await waitForAppReady(page);
    await expect.poll(() => themeState(page)).toMatchObject({ mode: 'dark' });
  });

  test('agents/new creates a custom agent that appears in the registry', async ({ page }) => {
    const agentId = `pw-researcher-${Date.now()}`;
    await openSettings(page, 'pw-settings-agent-new', '/settings/agents/new');

    // Page title dropped in the PanelPage migration; the Name field confirms the
    // agent editor mounted.
    await expect(page.getByRole('textbox', { name: 'Name' })).toBeVisible();
    await page.getByRole('textbox', { name: 'Name' }).fill('Playwright Researcher');
    await page.getByRole('textbox', { name: 'ID', exact: true }).fill(agentId);
    await page.getByLabel('Description').fill('Validates settings agent authoring in E2E.');
    await page.getByLabel('Model (optional)').selectOption('hint:reasoning');
    await page
      .getByLabel('System prompt (optional)')
      .fill('Prefer concise citations and explain uncertainty.');
    await page.getByRole('button', { name: 'Add tools' }).click();
    await page.getByRole('button', { name: /Allow all tools/ }).click();
    await page.getByRole('button', { name: 'Done', exact: true }).click();
    await page.getByRole('button', { name: 'Create agent' }).click();

    await expect(page).toHaveURL(/#\/settings\/agents$/);
    const agent = await callCoreRpc<{
      agent?: { id: string; model?: string; tool_allowlist?: string[] };
    }>('openhuman.agent_registry_get', { id: agentId });
    expect(agent.agent).toMatchObject({
      id: agentId,
      model: 'hint:reasoning',
      tool_allowlist: ['*'],
    });
  });

  test('retired task sources route lands on Connections', async ({ page }) => {
    await openSettings(page, 'pw-settings-task-sources', '/settings/task-sources');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections');
    await expect(page.getByRole('button', { name: 'Connections' }).first()).toBeVisible();
  });
});
