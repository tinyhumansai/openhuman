// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import {
  clickLabelContaining,
  clickSelector,
  clickText,
  textExists,
  waitForText,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-advanced-config';

async function readLocalStorageJson<T = unknown>(key: string): Promise<T | null> {
  return await browser.execute(storageKey => {
    const raw = window.localStorage.getItem(storageKey);
    return raw ? JSON.parse(raw) : null;
  }, key);
}

describe('Settings - Advanced Config', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('renders the developer options route and its advanced entries', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/developer-options');

    await waitForText('Advanced', 15_000);
    await waitForText('AI Configuration', 15_000);
    // 'Notification Routing' was removed as a top-level dev option in
    // PR #2550 — it now lives as a tab inside Settings → Notifications.
    await waitForText('Composio Routing (Direct Mode)', 15_000);
    await waitForText('About', 15_000);
  });

  it('persists notification routing settings through core RPC', async function () {
    this.timeout(60_000);
    const before = await callOpenhumanRpc('openhuman.notification_settings_get', {
      provider: 'gmail',
    });
    expect(before.ok).toBe(true);
    const initialEnabled = Boolean(before.result?.settings?.enabled);

    // /settings/notification-routing now redirects to
    // /settings/notifications#routing (the Routing tab on the tabbed
    // Notifications panel). Navigate to the tabbed panel directly and click
    // the Routing tab so we land on the same content the legacy path used to
    // render.
    await navigateViaHash('/settings/notifications');
    await clickText('Routing', 10_000);
    await waitForText('Notification Intelligence', 15_000);
    await clickSelector('input[type="checkbox"]');

    await browser.waitUntil(
      async () => {
        const after = await callOpenhumanRpc('openhuman.notification_settings_get', {
          provider: 'gmail',
        });
        return after.ok && Boolean(after.result?.settings?.enabled) === !initialEnabled;
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'notification routing did not persist' }
    );
  });

  it('persists composio trigger triage settings', async function () {
    this.timeout(60_000);
    const before = await callOpenhumanRpc('openhuman.config_get_composio_trigger_settings', {});
    expect(before.ok).toBe(true);

    await navigateViaHash('/settings/composio-triggers');
    await waitForText('Integration Triggers', 15_000);

    const disabledToolkitsInput = await browser.$('#disabled-toolkits');
    await disabledToolkitsInput.waitForExist({ timeout: 10_000 });
    await disabledToolkitsInput.setValue('gmail, slack');
    await clickText('Save', 10_000);
    await waitForText('Settings saved', 10_000);

    await browser.waitUntil(
      async () => {
        const after = await callOpenhumanRpc('openhuman.config_get_composio_trigger_settings', {});
        const result = after.result?.result ?? {};
        return (
          after.ok &&
          Array.isArray(result.triage_disabled_toolkits) &&
          result.triage_disabled_toolkits.includes('gmail') &&
          result.triage_disabled_toolkits.includes('slack')
        );
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'composio trigger settings did not persist' }
    );
  });

  it('persists autonomy max_actions_per_hour through core RPC', async function () {
    this.timeout(60_000);
    const before = await callOpenhumanRpc('openhuman.config_get_autonomy_settings', {});
    expect(before.ok).toBe(true);
    const current = before.result?.result?.max_actions_per_hour ?? 20;
    // Pick a value different from the current one so the save actually mutates state.
    const target = current === 250 ? 251 : 250;

    await navigateViaHash('/settings/autonomy');
    await waitForText('Agent autonomy', 15_000);

    const input = await browser.$('#autonomy-max-actions');
    await input.waitForExist({ timeout: 10_000 });
    await input.setValue(String(target));
    await clickText('Save', 10_000);
    await waitForText('Saved.', 10_000);

    await browser.waitUntil(
      async () => {
        const after = await callOpenhumanRpc('openhuman.config_get_autonomy_settings', {});
        return after.ok && after.result?.result?.max_actions_per_hour === target;
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'autonomy setting did not persist' }
    );
  });

  it('switches composio routing mode to direct and can return to backend mode', async function () {
    this.timeout(60_000);
    await navigateViaHash('/settings/composio-routing');
    await waitForText('Routing mode', 15_000);

    await clickLabelContaining('Direct (bring your own API key)');
    const apiKeyInput = await browser.$('#composio-api-key');
    await apiKeyInput.waitForExist({ timeout: 10_000 });
    await apiKeyInput.setValue('ck_live_e2e_composio_key');
    await clickText('Save', 10_000);
    if (await textExists('I understand, switch to Direct')) {
      await clickText('I understand, switch to Direct', 10_000);
    }

    await browser.waitUntil(
      async () => {
        const mode = await callOpenhumanRpc('openhuman.composio_get_mode', {});
        return (
          mode.ok &&
          mode.result?.result?.mode === 'direct' &&
          mode.result?.result?.api_key_set === true
        );
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'composio direct mode did not persist' }
    );

    const cleared = await callOpenhumanRpc('openhuman.composio_clear_api_key', {});
    expect(cleared.ok).toBe(true);
    const backend = await callOpenhumanRpc('openhuman.composio_get_mode', {});
    expect(backend.ok).toBe(true);
    expect(backend.result?.result?.mode).toBe('backend');
    expect(backend.result?.result?.api_key_set).toBe(false);
  });

  it('persists agent chat draft state to localStorage', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/agent-chat');

    await waitForText('Overrides', 15_000);

    // Use the native value setter + React change event to drive controlled
    // inputs. WebDriver's setValue clears the field but does not always
    // trigger React's synthetic onChange on controlled inputs.
    const setReactInput = async (selector: string, value: string) => {
      await browser.execute(
        (sel: string, val: string) => {
          const el = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(sel);
          if (!el) return;
          const setter = Object.getOwnPropertyDescriptor(
            el instanceof HTMLTextAreaElement
              ? window.HTMLTextAreaElement.prototype
              : window.HTMLInputElement.prototype,
            'value'
          )?.set;
          if (setter) setter.call(el, val);
          else el.value = val;
          el.dispatchEvent(new Event('input', { bubbles: true }));
          el.dispatchEvent(new Event('change', { bubbles: true }));
        },
        selector,
        value
      );
    };

    await setReactInput('input[placeholder="gpt-4o"]', 'gpt-4.1-mini');
    await setReactInput('input[placeholder="0.7"]', '0.2');
    await browser.pause(500);

    await browser.waitUntil(
      async () => {
        const payload = await readLocalStorageJson<{
          modelOverride?: string;
          temperature?: string;
          messages?: Array<{ role: string; text: string }>;
        }>('openhuman.settings.agentChat.history');
        return payload?.modelOverride === 'gpt-4.1-mini' && payload?.temperature === '0.2';
      },
      { timeout: 20_000, interval: 500, timeoutMsg: 'agent chat draft did not persist' }
    );
  });

  it('mounts the remaining advanced settings routes', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/local-model-debug');
    await waitForText('Local Model Debug', 15_000);

    await navigateViaHash('/settings/about');
    await waitForText('Software updates', 15_000);

    await navigateViaHash('/settings/llm');
    await waitForText('AI', 20_000);
    expect(
      (await textExists('Reasoning')) ||
        (await textExists('Cloud providers')) ||
        (await textExists('OpenHuman'))
    ).toBe(true);
  });
});
