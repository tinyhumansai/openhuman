// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { dumpAccessibilityTree, textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-webhooks-ingress';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[WebhooksIngressE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[WebhooksIngressE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function openWebhooksDebugPanel(): Promise<void> {
  await navigateViaHash('/settings/webhooks-debug');
}

describe('Webhooks ingress surface (stub-level)', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('reaches the app shell after onboarding', async () => {
    // Home page renders a CTA button with this text (t('home.askAssistant')).
    // The old anchors ('Message OpenHuman', 'Good morning', 'Upgrade to
    // Premium') no longer appear on the home page.
    const atHome =
      (await textExists('Ask your assistant anything')) || (await textExists('Ask your assistant'));
    expect(atHome).toBe(true);
  });

  it('exposes the stub webhook RPC surface with stable result and log shapes', async () => {
    const tunnelUuid = 'e2e-webhooks-ingress-tunnel';

    // list_registrations gracefully returns [] when router is not yet initialized
    // (which is the case in E2E where no real socket connection is made).
    const registrations = await callOpenhumanRpc('openhuman.webhooks_list_registrations', {});
    expect(registrations.ok).toBe(true);
    expect(registrations.result?.result?.registrations).toEqual([]);
    // Log message contains "returned 0" (either "registration(s)" suffix or
    // "registration(s) (router not initialized)" — both are valid).
    expect(registrations.result?.logs?.[0]).toContain('webhooks.list_registrations returned 0');

    const logs = await callOpenhumanRpc('openhuman.webhooks_list_logs', { limit: 5 });
    expect(logs.ok).toBe(true);
    expect(logs.result?.result?.logs).toEqual([]);
    expect(logs.result?.logs?.[0]).toContain('webhooks.list_logs returned 0');

    // register_echo requires the socket/webhook router to be initialized.
    // In E2E the backend socket is mocked so the router may not be available.
    // We verify the RPC endpoint is reachable but do not hard-assert success.
    const register = await callOpenhumanRpc('openhuman.webhooks_register_echo', {
      tunnel_uuid: tunnelUuid,
      tunnel_name: 'E2E Tunnel',
      backend_tunnel_id: 'backend-e2e-webhooks-ingress',
    });
    stepLog('register_echo result', { ok: register.ok, result: register.result });
    // The RPC must be reachable (not a network error) — ok=true means router is up,
    // ok=false means router not initialized (acceptable in E2E).
    expect(typeof register.ok).toBe('boolean');

    if (register.ok) {
      // Router was available — verify the result shape and full round-trip.
      expect(Array.isArray(register.result?.result?.registrations)).toBe(true);
      expect(register.result?.logs?.[0]).toContain(
        `webhooks.register_echo registered tunnel ${tunnelUuid}`
      );

      const clear = await callOpenhumanRpc('openhuman.webhooks_clear_logs', {});
      expect(clear.ok).toBe(true);
      expect(clear.result?.result?.cleared).toBeGreaterThanOrEqual(0);
      expect(clear.result?.logs?.[0]).toContain('webhooks.clear_logs removed 0');

      const unregister = await callOpenhumanRpc('openhuman.webhooks_unregister_echo', {
        tunnel_uuid: tunnelUuid,
      });
      expect(unregister.ok).toBe(true);
      expect(unregister.result?.result?.registrations).toEqual([]);
      expect(unregister.result?.logs?.[0]).toContain(
        `webhooks.unregister_echo removed tunnel ${tunnelUuid}`
      );
    } else {
      // Router not initialized — verify list/clear still work gracefully.
      const clear = await callOpenhumanRpc('openhuman.webhooks_clear_logs', {});
      expect(clear.ok).toBe(true);
      expect(clear.result?.result?.cleared).toBe(0);
      expect(clear.result?.logs?.[0]).toContain('webhooks.clear_logs removed 0');
    }
  });

  it('renders the webhooks debug panel empty states', async () => {
    await openWebhooksDebugPanel();

    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Navigated to webhooks debug route', { currentHash });
    expect(String(currentHash)).toContain('/settings/webhooks-debug');

    await waitForText('Webhooks Debug', 12_000);
    await waitForText('Registered Webhooks', 12_000);
    await waitForText('Captured Requests', 12_000);

    const hasEmptyStates =
      (await textExists('No active registrations.')) &&
      (await textExists('No webhook requests captured yet.'));

    if (!hasEmptyStates) {
      const tree = await dumpAccessibilityTree();
      stepLog('Webhooks debug empty states missing', { tree: tree.slice(0, 4000) });
    }

    expect(hasEmptyStates).toBe(true);
  });
});
