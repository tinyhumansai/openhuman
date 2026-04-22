/**
 * End-to-end: webhook tunnel CRUD round-trip (UI WebView → core JSON-RPC → mock backend).
 *
 * The webhook tunnel UI (Settings → Developer Options → Webhooks, plus the `/webhooks`
 * ComposeIO trigger history page) is a shipped, user-visible feature backed by the
 * `openhuman.webhooks_*` controller family registered in `src/openhuman/webhooks/schemas.rs`.
 * Prior to this spec there was no E2E coverage for the webhook path — only Rust-side unit
 * tests in `src/openhuman/webhooks/tests.rs` and the mock-backend tunnel CRUD endpoints
 * added in `scripts/mock-api-core.mjs` (`/webhooks/core*`).
 *
 * This spec validates the **authenticated** round-trip where the desktop shell's JSON-RPC
 * transport reaches the core sidecar, which in turn reaches the mock backend at
 * `/webhooks/core`. It is intentionally narrow: one coherent create → list → delete flow
 * that also surfaces the Webhooks page so the UI entry point does not silently regress.
 *
 * Auth model: `auth_store_session` is invoked implicitly by the web-layer deep link
 * listener (`desktopDeepLinkListener.ts → storeSession`). Webhook RPCs that require a
 * session token inherit that stored credential — no extra RPC priming is required here.
 *
 * Out of scope (tracked elsewhere):
 *  - `register_echo` / `list_registrations` / `clear_logs` — currently stub ops in
 *    `src/openhuman/webhooks/ops.rs` (no backend round-trip), covered by Rust unit tests.
 *  - ComposeIO history archive content — covered by `useComposeioTriggerHistory` hook
 *    unit tests and the core's ComposeIO handlers.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  navigateViaHash,
  waitForRequest,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[WebhooksTunnelE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[WebhooksTunnelE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

/**
 * Webhook ops build their RPC response with `RpcOutcome::single_log(...)`, which
 * `into_cli_compatible_json` serializes as `{ result, logs }` when at least one
 * log entry is present. Peel that wrapper off so assertions can target the
 * domain payload regardless of whether the handler attached logs — mirrors the
 * `.get("result").unwrap_or(outer)` pattern in `tests/json_rpc_e2e.rs`.
 */
function unwrapRpcValue<T = unknown>(raw: unknown): T | undefined {
  if (raw === null || raw === undefined) return undefined;
  if (typeof raw === 'object' && raw !== null && 'result' in (raw as Record<string, unknown>)) {
    const inner = (raw as { result?: unknown }).result;
    if (inner !== undefined) return inner as T;
  }
  return raw as T;
}

describe('Webhook tunnel CRUD (UI + core RPC + mock backend)', () => {
  before(async () => {
    await startMockServer();
    await resetMockBehavior();
    await waitForApp();
    clearRequestLog();
  });

  beforeEach(() => {
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates via bypass deep link and reaches the logged-in shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-webhooks-tunnel-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[WebhooksTunnelE2E]');

    const atHome =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Good morning')) ||
      (await textExists('Upgrade to Premium'));
    expect(atHome).toBe(true);
  });

  it('creates a tunnel → lists → deletes, with matching mock-backend traffic', async () => {
    // Wait for the deep-link listener's async `storeSession()` to settle before
    // exercising tunnel RPCs (webhooks ops require a stored session token).
    await browser.waitUntil(
      async () => {
        const probe = await callOpenhumanRpc('openhuman.webhooks_list_tunnels', {});
        return probe.ok;
      },
      {
        timeout: 15_000,
        interval: 500,
        timeoutMsg: 'Session did not settle: webhooks_list_tunnels never returned ok',
      }
    );

    // --- create ---------------------------------------------------------------
    clearRequestLog();
    const tunnelName = `e2e-tunnel-${Date.now()}`;
    const created = await callOpenhumanRpc('openhuman.webhooks_create_tunnel', {
      name: tunnelName,
      description: 'Created by webhooks-tunnel-flow E2E spec.',
    });
    if (!created.ok) {
      stepLog('webhooks_create_tunnel failed', created);
      stepLog('Mock request log at failure', getRequestLog());
    }
    expect(created.ok).toBe(true);
    const createdTunnel = unwrapRpcValue<{ id?: string; uuid?: string; name?: string }>(
      created.result
    );
    const tunnelId = createdTunnel?.id;
    expect(typeof tunnelId).toBe('string');
    expect((tunnelId as string).length).toBeGreaterThan(0);
    expect(createdTunnel?.name).toBe(tunnelName);

    const createReq = await waitForRequest(getRequestLog, 'POST', '/webhooks/core', 10_000);
    if (!createReq) {
      stepLog('No POST /webhooks/core observed', getRequestLog());
    }
    expect(createReq).toBeDefined();

    // --- list -----------------------------------------------------------------
    clearRequestLog();
    const listed = await callOpenhumanRpc('openhuman.webhooks_list_tunnels', {});
    if (!listed.ok) {
      stepLog('webhooks_list_tunnels failed', listed);
    }
    expect(listed.ok).toBe(true);
    const listedValue = unwrapRpcValue<Array<{ id?: string; name?: string }>>(listed.result);
    const tunnels = Array.isArray(listedValue) ? listedValue : [];
    const found = tunnels.find(t => t?.id === tunnelId);
    expect(found).toBeDefined();
    expect(found?.name).toBe(tunnelName);

    const listReq = await waitForRequest(getRequestLog, 'GET', '/webhooks/core', 10_000);
    expect(listReq).toBeDefined();

    // --- delete ---------------------------------------------------------------
    clearRequestLog();
    const deleted = await callOpenhumanRpc('openhuman.webhooks_delete_tunnel', { id: tunnelId });
    if (!deleted.ok) {
      stepLog('webhooks_delete_tunnel failed', deleted);
    }
    expect(deleted.ok).toBe(true);

    const deleteReq = await waitForRequest(
      getRequestLog,
      'DELETE',
      `/webhooks/core/${tunnelId}`,
      10_000
    );
    if (!deleteReq) {
      stepLog('No DELETE /webhooks/core/<id> observed', getRequestLog());
    }
    expect(deleteReq).toBeDefined();

    // --- post-delete list confirms removal ------------------------------------
    clearRequestLog();
    const relisted = await callOpenhumanRpc('openhuman.webhooks_list_tunnels', {});
    expect(relisted.ok).toBe(true);
    const relistedValue = unwrapRpcValue<Array<{ id?: string }>>(relisted.result);
    const stillPresent = (Array.isArray(relistedValue) ? relistedValue : []).some(
      t => t?.id === tunnelId
    );
    expect(stillPresent).toBe(false);
  });

  it('Webhooks page loads (ComposeIO trigger history surface)', async () => {
    await navigateViaHash('/webhooks');

    await browser.waitUntil(
      async () => {
        return (
          (await textExists('ComposeIO Triggers')) ||
          (await textExists('ComposeIO')) ||
          (await textExists('Archive')) ||
          (await textExists('Refresh'))
        );
      },
      { timeout: 10_000, interval: 500, timeoutMsg: 'Webhooks page markers did not appear' }
    );

    if (supportsExecuteScript()) {
      const hash = await browser.execute(() => window.location.hash);
      expect(String(hash)).toContain('/webhooks');
    }

    const visible =
      (await textExists('ComposeIO Triggers')) ||
      (await textExists('ComposeIO')) ||
      (await textExists('Archive')) ||
      (await textExists('Refresh'));
    if (!visible) {
      stepLog('Webhooks page markers missing');
      await dumpAccessibilityTree();
      stepLog('Mock request log', getRequestLog());
    }
    expect(visible).toBe(true);
  });
});
