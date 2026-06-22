import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
} from '../helpers/core-rpc';

async function emulateTauriRuntime(page: Page): Promise<void> {
  await page.evaluate(() => {
    const win = window as typeof window & {
      isTauri?: boolean;
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    win.isTauri = true;
    win.__TAURI_INTERNALS__ = win.__TAURI_INTERNALS__ ?? {};
    win.__TAURI_INTERNALS__.invoke = win.__TAURI_INTERNALS__.invoke ?? (async () => null);
  });
}

async function waitForAdvancedRouteReady(page: Page): Promise<void> {
  await page.waitForSelector('#root', { state: 'attached' });
  await expect
    .poll(async () => {
      const text = await page
        .locator('#root')
        .innerText()
        .catch(() => '');
      return text.trim().length;
    })
    .toBeGreaterThan(20);
  await expect(page.getByText(/Select a Runtime|Connect to Your Runtime/)).toHaveCount(0);
}

async function gotoSettingsRoute(page: Page, hash: string): Promise<void> {
  await page.goto(`/#${hash}`);
  await waitForAdvancedRouteReady(page);
  await dismissWalkthroughIfPresent(page);
}

function providerEnabledToggle(
  page: Page,
  providerName: 'gmail' | 'slack' | 'discord' | 'whatsapp'
): Locator {
  const providerOrder = ['gmail', 'slack', 'discord', 'whatsapp'] as const;
  const index = providerOrder.indexOf(providerName);
  if (index < 0) {
    throw new Error(`Unsupported provider row: ${providerName}`);
  }
  return page.getByRole('checkbox', { name: 'Enabled' }).nth(index);
}

test.describe('Settings - Advanced Config', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-settings-advanced-user');
    await emulateTauriRuntime(page);
  });

  test('renders the developer options route and its advanced entries', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/developer-options');

    // Panel title dropped in the PanelPage migration; the panel is confirmed by
    // its diagnostics entries below.
    // Developer Options is debug-only now: user-facing sections (AI, Integrations…)
    // live on their section pages, so Developer Options surfaces diagnostics entries.
    // The two-pane sidebar may also surface these ids, so scope to the first match.
    await expect(page.getByTestId('settings-nav-memory-debug').first()).toBeVisible();
    await expect(page.getByTestId('settings-nav-event-log').first()).toBeVisible();
    await expect(page.getByTestId('settings-nav-build-info').first()).toBeVisible();
  });

  test('persists notification routing settings through core RPC', async ({ page }) => {
    const before = await callCoreRpc<{ settings?: { enabled?: boolean } }>(
      'openhuman.notification_settings_get',
      { provider: 'gmail' }
    );
    const initialEnabled = Boolean(before.settings?.enabled);

    await gotoSettingsRoute(page, '/settings/notifications');
    await page.getByRole('tab', { name: 'Routing' }).click();
    await expect(page.getByText('Notification Intelligence')).toBeVisible();

    await providerEnabledToggle(page, 'gmail').click();

    await expect
      .poll(async () => {
        const after = await callCoreRpc<{ settings?: { enabled?: boolean } }>(
          'openhuman.notification_settings_get',
          { provider: 'gmail' }
        );
        return Boolean(after.settings?.enabled);
      })
      .toBe(!initialEnabled);
  });

  test('persists composio trigger triage settings', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/composio-triggers');

    await expect(page.getByText('Integration Triggers')).toBeVisible();
    await page.locator('#disabled-toolkits').fill('gmail, slack');
    await page.getByRole('button', { name: 'Save' }).click();
    await expect(page.getByText('Settings saved')).toBeVisible();

    await expect
      .poll(async () => {
        const after = await callCoreRpc<{ result?: { triage_disabled_toolkits?: string[] } }>(
          'openhuman.config_get_composio_trigger_settings',
          {}
        );
        const disabled = after.result?.triage_disabled_toolkits ?? [];
        return disabled.includes('gmail') && disabled.includes('slack');
      })
      .toBe(true);
  });

  test('persists autonomy max_actions_per_hour through core RPC', async ({ page }) => {
    const before = await callCoreRpc<{ result?: { max_actions_per_hour?: number } }>(
      'openhuman.config_get_autonomy_settings',
      {}
    );
    const current = before.result?.max_actions_per_hour ?? 20;
    const target = current === 250 ? 251 : 250;

    // /settings/autonomy redirects to Agent access, which hosts the autonomy
    // rate-limit section (Max actions per hour).
    await gotoSettingsRoute(page, '/settings/autonomy');

    await expect(page.getByRole('heading', { name: 'Max actions per hour' })).toBeVisible();
    await page.locator('#autonomy-max-actions').fill(String(target));
    await page.getByRole('button', { name: 'Save' }).click();
    await expect(page.getByText('Saved.')).toBeVisible();

    await expect
      .poll(async () => {
        const after = await callCoreRpc<{ result?: { max_actions_per_hour?: number } }>(
          'openhuman.config_get_autonomy_settings',
          {}
        );
        return after.result?.max_actions_per_hour;
      })
      .toBe(target);
  });

  test('switches composio routing mode to direct and can return to backend mode', async ({
    page,
  }) => {
    await gotoSettingsRoute(page, '/settings/composio-routing');

    await expect(page.getByText('Routing mode')).toBeVisible();
    await page.getByLabel(/Direct/).check();
    await page.locator('#composio-api-key').fill('ck_live_e2e_composio_key');
    await page.getByRole('button', { name: 'Save' }).click();

    const confirm = page.getByRole('button', { name: 'I understand, switch to Direct' });
    if (await confirm.isVisible().catch(() => false)) {
      await confirm.click();
    }

    await expect
      .poll(async () => {
        const mode = await callCoreRpc<{ result?: { mode?: string; api_key_set?: boolean } }>(
          'openhuman.composio_get_mode',
          {}
        );
        return { mode: mode.result?.mode ?? null, apiKeySet: Boolean(mode.result?.api_key_set) };
      })
      .toEqual({ mode: 'direct', apiKeySet: true });

    await callCoreRpc('openhuman.composio_clear_api_key', {});
    const backend = await callCoreRpc<{ result?: { mode?: string; api_key_set?: boolean } }>(
      'openhuman.composio_get_mode',
      {}
    );
    expect(backend.result?.mode).toBe('backend');
    expect(backend.result?.api_key_set).toBe(false);
  });

  test('persists agent chat draft state to localStorage', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/agent-chat');

    // The panel's description copy also contains the word "overrides", so scope
    // to the section heading to avoid a strict-mode match on both.
    await expect(page.getByRole('heading', { name: 'Overrides' })).toBeVisible();
    await page.getByPlaceholder('gpt-4o').fill('gpt-4.1-mini');
    await page.getByPlaceholder('0.7').fill('0.2');

    await expect
      .poll(async () =>
        page.evaluate(() => {
          const raw = window.localStorage.getItem('openhuman.settings.agentChat.history');
          if (!raw) return null;
          const payload = JSON.parse(raw) as { modelOverride?: string; temperature?: string };
          return {
            modelOverride: payload.modelOverride ?? null,
            temperature: payload.temperature ?? null,
          };
        })
      )
      .toEqual({ modelOverride: 'gpt-4.1-mini', temperature: '0.2' });
  });

  test('mounts the remaining advanced settings routes', async ({ page }) => {
    await gotoSettingsRoute(page, '/settings/local-model-debug');
    // The two-pane sidebar also renders this label, so scope to the first match.
    await expect(page.getByText('Local Model Debug').first()).toBeVisible();

    await gotoSettingsRoute(page, '/settings/about');
    // The About description copy also contains "software updates"; match the
    // section label exactly to avoid a strict-mode violation.
    await expect(page.getByText('Software updates', { exact: true })).toBeVisible();

    // /settings/llm now redirects to the Connections page (LLM moved there).
    await gotoSettingsRoute(page, '/settings/llm');
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections');
    await expect(page.getByText(/Reasoning|Cloud providers|OpenHuman/).first()).toBeVisible();
  });
});
