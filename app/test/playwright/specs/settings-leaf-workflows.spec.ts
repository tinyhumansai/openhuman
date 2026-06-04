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

async function themeState(page: Page): Promise<{ mode?: string; tabBarLabels?: string }> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => { theme?: { mode?: string; tabBarLabels?: string } };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store?.getState?.().theme ?? {};
  });
}

async function persistedThemeState(page: Page): Promise<{ mode?: string; tabBarLabels?: string }> {
  return page.evaluate(() => {
    const raw = localStorage.getItem('persist:theme');
    if (!raw) return {};
    try {
      const parsed = JSON.parse(raw) as Record<string, string>;
      return {
        mode: parsed.mode ? JSON.parse(parsed.mode) : undefined,
        tabBarLabels: parsed.tabBarLabels ? JSON.parse(parsed.tabBarLabels) : undefined,
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
  test('appearance theme mode and tab bar label preference persist in app state', async ({
    page,
  }) => {
    await openSettings(page, 'pw-settings-appearance', '/settings/appearance');

    await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible();
    await page.getByRole('radio', { name: /Dark/ }).click();
    const labelSwitch = page.getByRole('switch', { name: /Always show labels/ });
    if ((await labelSwitch.getAttribute('aria-checked')) !== 'true') {
      await labelSwitch.click();
    }

    await expect
      .poll(() => themeState(page))
      .toMatchObject({ mode: 'dark', tabBarLabels: 'always' });
    await expect
      .poll(() => persistedThemeState(page))
      .toMatchObject({ mode: 'dark', tabBarLabels: 'always' });

    await page.reload();
    await waitForAppReady(page);
    await expect
      .poll(() => themeState(page))
      .toMatchObject({ mode: 'dark', tabBarLabels: 'always' });
  });

  test('embeddings custom endpoint setup writes provider, model, and dimensions', async ({
    page,
  }) => {
    await openSettings(page, 'pw-settings-embeddings', '/settings/embeddings');

    await expect(page.getByRole('heading', { name: 'Embeddings' })).toBeVisible();
    await page.getByRole('radio', { name: /Custom/i }).click();

    await expect(page.getByRole('heading', { name: /Set up/i })).toBeVisible();
    await page
      .getByPlaceholder('https://your-endpoint.com/v1')
      .fill('http://127.0.0.1:18473/openai/v1');
    await page.getByPlaceholder('text-embedding-3-small').fill('e2e-embedding-model');
    await page.getByPlaceholder('1024').fill('64');
    await page.getByRole('button', { name: 'Save & switch' }).click();

    const wipe = page.getByRole('button', { name: 'Wipe & apply' });
    await expect(wipe).toBeVisible({ timeout: 15_000 });
    await wipe.click();

    await expect(page.getByText('Saved.')).toBeVisible({ timeout: 15_000 });
    await expect
      .poll(async () => {
        const raw = await callCoreRpc<any>('openhuman.embeddings_get_settings', {});
        const settings =
          unwrap<{ provider?: string; model?: string; dimensions?: number }>(raw) ?? {};
        return {
          provider: settings.provider,
          model: settings.model,
          dimensions: settings.dimensions,
        };
      })
      .toEqual({
        provider: 'custom:http://127.0.0.1:18473/openai/v1',
        model: 'e2e-embedding-model',
        dimensions: 64,
      });
  });

  test('agents/new creates a custom agent that appears in the registry', async ({ page }) => {
    const agentId = `pw-researcher-${Date.now()}`;
    await openSettings(page, 'pw-settings-agent-new', '/settings/agents/new');

    await expect(page.getByRole('heading', { name: 'New agent' })).toBeVisible();
    await page.getByRole('textbox', { name: 'Name' }).fill('Playwright Researcher');
    await page.getByRole('textbox', { name: /ID Lowercase/ }).fill(agentId);
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

  test('task sources surface the web harness guard while preserving the create form', async ({
    page,
  }) => {
    const name = `Playwright Issues ${Date.now()}`;
    await openSettings(page, 'pw-settings-task-sources', '/settings/task-sources');

    await expect(page.getByTestId('task-sources-panel')).toBeVisible();
    await expect(page.getByText('Not running in Tauri')).toBeVisible();
    await page.getByLabel('Provider').selectOption('github');
    await page.getByLabel('Name (optional)').fill(name);
    await page.getByLabel('Repository (owner/name, optional)').fill('tinyhumansai/openhuman');
    await page.getByLabel('Labels (comma-separated)').fill('e2e, regression');
    await expect(page.getByRole('button', { name: 'Add source' })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Preview' })).toBeEnabled();
  });
});
