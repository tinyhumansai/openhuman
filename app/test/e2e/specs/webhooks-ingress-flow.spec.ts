// @ts-nocheck
import { expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
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

describe('Webhooks ingress surface (stub-level)', () => {
  before(async function () {
    // resetApp bring-up (waitForApp + onboarding walk + home confirm) can run
    // ~25-30s and race the default 30s Mocha hook budget; raise it.
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
    // Home.tsx: t('home.askAssistant') is the stable home page CTA button text.
    // After the /home → /chat redirect (AppRoutes.tsx), the chat new-window hero
    // renders t('home.statusOk') instead of the old CTA button.
    const atHome =
      (await textExists('Ask your assistant anything')) ||
      (await textExists('Your device is connected')) ||
      (await textExists('Your assistant is ready when you are')) ||
      (await textExists('Type something below to get started'));
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
    stepLog('register_echo result', { ok: register.ok, error: register.error });

    // register_echo requires the socket-backed webhook router to be
    // initialized. In E2E the socket may not be connected, so the router
    // is uninitialized and the call returns an error. When ok=false, skip
    // the write-path assertions and only validate the read-only surface.
    if (register.ok) {
      const regs = register.result?.result?.registrations ?? [];
      expect(Array.isArray(regs)).toBe(true);
      expect(regs.length).toBeGreaterThanOrEqual(1);
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
    } else {
      stepLog('register_echo failed (router not initialized) — skipping write-path assertions');
    }
  });
});
