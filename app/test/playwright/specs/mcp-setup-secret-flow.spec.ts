import { expect, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage } from '../helpers/core-rpc';

test.describe('MCP Setup — Secret Collection Flow', () => {
  test('SecretPromptDialog appears on event, collects input, and submits without leaking to agent context', async ({
    page,
  }) => {
    await bootRuntimeReadyGuestPage(page);

    // Intercept all RPC calls to track what gets sent.
    // Mock mcp_setup_submit_secret since there's no real SecretRef in core memory.
    const rpcCalls: Array<{ method: string; params: Record<string, unknown> }> = [];
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      rpcCalls.push({ method: body.method, params: body.params });
      if (body.method === 'openhuman.mcp_setup_submit_secret') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: { ref: body.params.ref_id, fulfilled: true },
          }),
        });
      } else {
        await route.continue();
      }
    });

    // Simulate the core publishing a McpSetupSecretRequested event.
    // In production this comes via the socket → window event bridge.
    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: {
            refId: 'secret://aabbccdd1122',
            keyName: 'NOTION_API_KEY',
            prompt: 'Enter your Notion integration token to connect.',
          },
        })
      );
    });

    // The dialog should appear
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Should show the key name and prompt
    await expect(dialog.locator('code')).toContainText('NOTION_API_KEY');
    await expect(dialog).toContainText('Enter your Notion integration token to connect.');

    // The input should be type=password (not leaking visually)
    const input = dialog.locator('input[type="password"]');
    await expect(input).toBeVisible();

    // Type a secret value
    await input.fill('ntn_secret_test_value_12345');

    // Submit
    const submitButton = dialog.locator('button[type="submit"]');
    await expect(submitButton).toBeEnabled();
    await submitButton.click();

    // Dialog should dismiss
    await expect(dialog).not.toBeVisible({ timeout: 5_000 });

    // Verify the RPC call was made with the correct ref but the value
    // only goes to submit_secret (never to any agent-facing method)
    const submitCall = rpcCalls.find(c => c.method === 'openhuman.mcp_setup_submit_secret');
    expect(submitCall).toBeTruthy();
    expect(submitCall!.params.ref_id).toBe('secret://aabbccdd1122');
    expect(submitCall!.params.value).toBe('ntn_secret_test_value_12345');

    // Critically: no agent-facing RPC (mcp_setup_test_connection,
    // mcp_setup_install_and_connect, or any chat/thread method) should
    // contain the raw secret value in its params.
    const agentFacingCalls = rpcCalls.filter(
      c =>
        c.method !== 'openhuman.mcp_setup_submit_secret' &&
        c.method !== 'openhuman.auth_store_session' &&
        c.method !== 'openhuman.auth_clear_session' &&
        c.method !== 'openhuman.config_set_onboarding_completed'
    );
    for (const call of agentFacingCalls) {
      const serialized = JSON.stringify(call.params);
      expect(serialized).not.toContain('ntn_secret_test_value_12345');
    }
  });

  test('cancel does not submit the secret', async ({ page }) => {
    await bootRuntimeReadyGuestPage(page);

    const rpcCalls: Array<{ method: string }> = [];
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      rpcCalls.push({ method: body.method });
      await route.continue();
    });

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: { refId: 'secret://cancel123456', keyName: 'API_KEY', prompt: 'Enter key' },
        })
      );
    });

    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Click cancel
    const cancelButton = dialog.getByRole('button', { name: /cancel/i });
    await cancelButton.click();

    // Dialog should dismiss
    await expect(dialog).not.toBeVisible({ timeout: 3_000 });

    // No submit_secret call should have been made
    const submitCalls = rpcCalls.filter(c => c.method === 'openhuman.mcp_setup_submit_secret');
    expect(submitCalls).toHaveLength(0);
  });

  test('secret input uses password masking by default', async ({ page }) => {
    await bootRuntimeReadyGuestPage(page);

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: { refId: 'secret://mask123456', keyName: 'TOKEN', prompt: '' },
        })
      );
    });

    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Input is password-type (masked)
    const input = dialog.locator('#mcp-setup-secret-input');
    await expect(input).toHaveAttribute('type', 'password');

    // Toggle to show
    const showButton = dialog.getByRole('button', { name: /show/i });
    await showButton.click();
    await expect(input).toHaveAttribute('type', 'text');

    // Toggle back to hide
    const hideButton = dialog.getByRole('button', { name: /hide/i });
    await hideButton.click();
    await expect(input).toHaveAttribute('type', 'password');
  });
});
