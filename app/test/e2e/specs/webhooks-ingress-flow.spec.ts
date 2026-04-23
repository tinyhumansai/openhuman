// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  navigateToSettings,
  navigateViaHash,
} from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[WebhooksIngressE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[WebhooksIngressE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function openWebhooksDebugPanel(): Promise<void> {
  if (supportsExecuteScript()) {
    await navigateViaHash('/settings/webhooks-debug');
    return;
  }

  await navigateToSettings();
  await clickText('Developer Options', 12_000);
  await clickText('Webhooks', 12_000);
}

describe('Webhooks ingress surface (stub-level)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and reaches the app shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-webhooks-ingress-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[WebhooksIngressE2E]');

    const atHome =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Good morning')) ||
      (await textExists('Upgrade to Premium'));
    expect(atHome).toBe(true);
  });

  it('exposes the stub webhook RPC surface with stable result and log shapes', async () => {
    const tunnelUuid = 'e2e-webhooks-ingress-tunnel';

    const registrations = await callOpenhumanRpc('openhuman.webhooks_list_registrations', {});
    expect(registrations.ok).toBe(true);
    expect(registrations.result?.result?.registrations).toEqual([]);
    expect(registrations.result?.logs?.[0]).toContain('webhooks.list_registrations returned 0');

    const logs = await callOpenhumanRpc('openhuman.webhooks_list_logs', { limit: 5 });
    expect(logs.ok).toBe(true);
    expect(logs.result?.result?.logs).toEqual([]);
    expect(logs.result?.logs?.[0]).toContain('webhooks.list_logs returned 0');

    const register = await callOpenhumanRpc('openhuman.webhooks_register_echo', {
      tunnel_uuid: tunnelUuid,
      tunnel_name: 'E2E Tunnel',
      backend_tunnel_id: 'backend-e2e-webhooks-ingress',
    });
    expect(register.ok).toBe(true);
    expect(register.result?.result?.registrations).toEqual([]);
    expect(register.result?.logs?.[0]).toContain(
      `webhooks.register_echo registered tunnel ${tunnelUuid}`
    );

    const clear = await callOpenhumanRpc('openhuman.webhooks_clear_logs', {});
    expect(clear.ok).toBe(true);
    expect(clear.result?.result?.cleared).toBe(0);
    expect(clear.result?.logs?.[0]).toContain('webhooks.clear_logs removed 0');

    const unregister = await callOpenhumanRpc('openhuman.webhooks_unregister_echo', {
      tunnel_uuid: tunnelUuid,
    });
    expect(unregister.ok).toBe(true);
    expect(unregister.result?.result?.registrations).toEqual([]);
    expect(unregister.result?.logs?.[0]).toContain(
      `webhooks.unregister_echo removed tunnel ${tunnelUuid}`
    );
  });

  it('renders the webhooks debug panel empty states', async () => {
    await openWebhooksDebugPanel();

    if (supportsExecuteScript()) {
      const currentHash = await browser.execute(() => window.location.hash);
      stepLog('Navigated to webhooks debug route', { currentHash });
      expect(String(currentHash)).toContain('/settings/webhooks-debug');
    }

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
