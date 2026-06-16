// @ts-nocheck
/**
 * MCP Setup Secret Dialog — desktop E2E (Appium / WDIO).
 *
 * Verifies that when the core publishes a `McpSetupSecretRequested` event,
 * the SecretPromptDialog appears in the native desktop WebView, collects
 * the user's input via a masked password field, and submits it only
 * through the `mcp_setup_submit_secret` RPC — never exposing the raw
 * value to any agent-facing channel.
 *
 * This is the Appium counterpart to the Playwright spec at
 * `test/playwright/specs/mcp-setup-secret-flow.spec.ts`. Both cover the
 * same functional contract; this one exercises it in the actual Tauri
 * desktop shell via CEF/WebView.
 */
import { waitForApp } from '../helpers/app-helpers';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-mcp-secret-dialog';

describe('MCP Setup — Secret Dialog', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('dialog renders on McpSetupSecretRequested event', async () => {
    // Dispatch the event that the socket service forwards from core
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: {
            refId: 'secret://e2e_test_ref1',
            keyName: 'TEST_API_KEY',
            prompt: 'Enter your test API key to verify the dialog.',
          },
        })
      );
    });

    // Wait for the dialog to appear
    const dialog = await $('[role="dialog"]');
    await dialog.waitForDisplayed({ timeout: 5_000 });

    // Verify key name and prompt text are shown
    const keyNameEl = await dialog.$('code');
    const keyNameText = await keyNameEl.getText();
    expect(keyNameText).toContain('TEST_API_KEY');

    const dialogText = await dialog.getText();
    expect(dialogText).toContain('Enter your test API key to verify the dialog.');
  });

  it('input is masked (type=password) by default', async () => {
    const input = await $('#mcp-setup-secret-input');
    const type = await input.getAttribute('type');
    expect(type).toBe('password');
  });

  it('show/hide toggle changes input type', async () => {
    const input = await $('#mcp-setup-secret-input');
    const dialog = await $('[role="dialog"]');

    // Find and click the "Show" button
    const showBtn = await dialog.$('button*=Show');
    await showBtn.click();
    expect(await input.getAttribute('type')).toBe('text');

    // Toggle back
    const hideBtn = await dialog.$('button*=Hide');
    await hideBtn.click();
    expect(await input.getAttribute('type')).toBe('password');
  });

  it('submit button is disabled when input is empty', async () => {
    const submitBtn = await $('[role="dialog"] button[type="submit"]');
    expect(await submitBtn.isEnabled()).toBe(false);
  });

  it('cancel dismisses the dialog without submitting', async () => {
    // Click cancel
    const dialog = await $('[role="dialog"]');
    const cancelBtn = await dialog.$('button*=Cancel');
    await cancelBtn.click();

    // Dialog should not be displayed
    await dialog.waitForDisplayed({ timeout: 3_000, reverse: true });
  });

  it('submit sends secret only to mcp_setup_submit_secret RPC', async () => {
    // Re-trigger a new dialog
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: {
            refId: 'secret://e2e_test_ref2',
            keyName: 'SECOND_KEY',
            prompt: 'Second key prompt.',
          },
        })
      );
    });

    const dialog = await $('[role="dialog"]');
    await dialog.waitForDisplayed({ timeout: 5_000 });

    // Intercept outgoing RPC calls by patching the fetch in the page
    await browser.execute(() => {
      (window as any).__e2eRpcLog = [];
      const origFetch = window.fetch;
      window.fetch = async function (...args: any[]) {
        const [url, opts] = args;
        if (typeof url === 'string' && url.includes('/rpc') && opts?.body) {
          try {
            const body = JSON.parse(opts.body);
            (window as any).__e2eRpcLog.push({ method: body.method, params: body.params });
          } catch (_e) {
            /* intentionally empty — best-effort logging */
          }
        }
        return origFetch.apply(this, args);
      };
    });

    // Type a value and submit
    const input = await $('#mcp-setup-secret-input');
    await input.setValue('e2e_super_secret_value');

    const submitBtn = await dialog.$('button[type="submit"]');
    await submitBtn.click();

    // Give it a moment to process
    await browser.pause(1_000);

    // Retrieve the RPC log
    const rpcLog = (await browser.execute(() => (window as any).__e2eRpcLog)) as Array<{
      method: string;
      params: Record<string, unknown>;
    }>;

    // The submit_secret call should carry the ref and value
    const submitCall = rpcLog.find(c => c.method === 'openhuman.mcp_setup_submit_secret');
    expect(submitCall).toBeDefined();
    expect(submitCall!.params.ref_id).toBe('secret://e2e_test_ref2');
    expect(submitCall!.params.value).toBe('e2e_super_secret_value');

    // No other RPC call should contain the raw secret
    const otherCalls = rpcLog.filter(c => c.method !== 'openhuman.mcp_setup_submit_secret');
    for (const call of otherCalls) {
      const serialized = JSON.stringify(call.params);
      expect(serialized).not.toContain('e2e_super_secret_value');
    }

    // Cleanup: restore original fetch
    await browser.execute(() => {
      delete (window as any).__e2eRpcLog;
    });
  });
});
