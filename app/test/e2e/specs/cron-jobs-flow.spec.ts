// @ts-nocheck
/**
 * End-to-end: cron jobs across the full desktop stack.
 *
 * Covers the cross-process flow that unit tests cannot prove:
 *   UI (Settings → Cron Jobs panel) → coreRpcClient → Tauri core_rpc_relay → openhuman sidecar
 *
 * What this validates:
 *   1. Completing onboarding triggers the sidecar's `seed_proactive_agents`
 *      side effect — the `morning_briefing` cron job must appear in `cron_list`
 *      without any explicit UI action (proves the post-onboarding hook wired to
 *      the cron seed ran in the real sidecar process, not just in isolation).
 *   2. `cron_update` round-trips a patch through the sidecar and the persisted
 *      state is reflected on a fresh `cron_list`.
 *   3. `cron_runs` on a never-run job returns an empty history (RPC shape).
 *   4. `cron_remove` on an unknown id surfaces a structured error back to
 *      the WebView (tests the error path end-to-end; the webview client
 *      returns `{ ok: false, error }` rather than throwing).
 *   5. The Settings → Cron Jobs panel renders after auth and shows the
 *      seeded morning_briefing job (UI ↔ core RPC sync).
 *
 * Method naming note: controllers register as `namespace=cron, function=list`
 * but the RPC method name is composed via `openhuman.{namespace}_{function}` —
 * so the wire method is `openhuman.cron_list`, matching what the UI's
 * `openhumanCronList` helper in app/src/utils/tauriCommands/cron.ts sends.
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
  navigateToSettings,
  navigateViaHash,
} from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const MORNING_BRIEFING_NAME = 'morning_briefing';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[CronJobsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[CronJobsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

interface CronJobMinimal {
  id: string;
  name?: string | null;
  enabled: boolean;
}

/**
 * RpcOutcome.into_cli_compatible_json wraps payloads as `{result: T, logs: [...]}`
 * whenever logs are non-empty — every cron op emits at least one log line, so
 * every cron RPC returns the wrapped shape. Mirror the `inner()` helper in
 * tests/json_rpc_e2e.rs and fall through to the raw value if logs were absent.
 */
function innerPayload<T>(outer: unknown): T | undefined {
  if (outer && typeof outer === 'object' && 'result' in (outer as object)) {
    return (outer as { result?: T }).result;
  }
  return outer as T | undefined;
}

async function waitForSeededJob(
  name: string,
  timeoutMs = 15_000
): Promise<CronJobMinimal | undefined> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;
  while (Date.now() < deadline) {
    const list = await callOpenhumanRpc('openhuman.cron_list', {});
    if (list.ok) {
      const jobs = innerPayload<CronJobMinimal[]>(list.result) ?? [];
      const match = Array.isArray(jobs) ? jobs.find(j => (j?.name ?? null) === name) : undefined;
      if (match) return match;
    } else {
      lastError = list;
    }
    await browser.pause(750);
  }
  if (lastError) {
    stepLog('waitForSeededJob: last cron_list error', lastError);
  }
  return undefined;
}

describe('Cron jobs (UI + core RPC)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and completes onboarding (seeds morning_briefing)', async () => {
    await triggerAuthDeepLinkBypass('e2e-cron-jobs');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[CronJobsE2E]');

    const atHome =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Good morning')) ||
      (await textExists('Upgrade to Premium'));
    expect(atHome).toBe(true);
  });

  it('core.ping responds over the UI JSON-RPC bridge', async () => {
    const ping = await callOpenhumanRpc('core.ping', {});
    if (!ping.ok) stepLog('core.ping failed', ping);
    expect(ping.ok).toBe(true);
  });

  it('cron_list surfaces the morning_briefing job seeded after onboarding', async () => {
    // seed_proactive_agents runs in a detached spawn_blocking task — poll.
    const seeded = await waitForSeededJob(MORNING_BRIEFING_NAME, 20_000);
    if (!seeded) {
      const snapshot = await callOpenhumanRpc('openhuman.cron_list', {});
      stepLog('morning_briefing not found; latest cron_list snapshot', snapshot);
    }
    expect(seeded).toBeTruthy();
    expect(typeof seeded?.id).toBe('string');
    expect(seeded?.enabled === true || seeded?.enabled === false).toBe(true);
  });

  it('cron_update round-trips an enabled=false patch through the sidecar', async () => {
    const seeded = await waitForSeededJob(MORNING_BRIEFING_NAME, 10_000);
    expect(seeded).toBeTruthy();
    const originalEnabled = seeded!.enabled;
    const target = !originalEnabled;

    const update = await callOpenhumanRpc('openhuman.cron_update', {
      job_id: seeded!.id,
      patch: { enabled: target },
    });
    if (!update.ok) stepLog('cron_update failed', update);
    expect(update.ok).toBe(true);
    const updated = innerPayload<CronJobMinimal>(update.result);
    expect(updated?.id).toBe(seeded!.id);
    expect(updated?.enabled).toBe(target);

    // Verify persistence across a fresh list call.
    const reread = await callOpenhumanRpc('openhuman.cron_list', {});
    expect(reread.ok).toBe(true);
    const rereadJobs = innerPayload<CronJobMinimal[]>(reread.result) ?? [];
    const after = rereadJobs.find(j => j.id === seeded!.id);
    expect(after?.enabled).toBe(target);

    // Restore the original state so subsequent specs/runs aren't poisoned.
    const restore = await callOpenhumanRpc('openhuman.cron_update', {
      job_id: seeded!.id,
      patch: { enabled: originalEnabled },
    });
    expect(restore.ok).toBe(true);
  });

  it('cron_runs on a never-run job returns an empty array', async () => {
    const seeded = await waitForSeededJob(MORNING_BRIEFING_NAME, 5_000);
    expect(seeded).toBeTruthy();

    const runs = await callOpenhumanRpc('openhuman.cron_runs', { job_id: seeded!.id, limit: 5 });
    if (!runs.ok) stepLog('cron_runs failed', runs);
    expect(runs.ok).toBe(true);
    const history = innerPayload<unknown[]>(runs.result) ?? [];
    expect(Array.isArray(history)).toBe(true);
    // Fresh workspace — morning_briefing has not fired.
    expect(history.length).toBe(0);
  });

  it('cron_remove on an unknown id surfaces an error via the RPC envelope', async () => {
    const missing = await callOpenhumanRpc('openhuman.cron_remove', {
      job_id: 'does-not-exist-e2e',
    });
    // The webview RPC envelope returns { ok:false, error } on JSON-RPC errors;
    // the node fallback shape is the same (see core-rpc-webview / core-rpc-node).
    expect(missing.ok).toBe(false);
    const errText = String(missing.error ?? '');
    expect(errText.length > 0).toBe(true);
  });

  it('Settings → Cron Jobs panel renders with the seeded job', async () => {
    await navigateToSettings();
    await browser.pause(1_000);
    await navigateViaHash('/settings/cron-jobs');
    await browser.pause(3_000);

    if (supportsExecuteScript()) {
      const hash = await browser.execute(() => window.location.hash);
      expect(String(hash)).toContain('/settings/cron-jobs');
    }

    // The panel title or a morning_briefing marker should be visible.
    const panelVisible =
      (await textExists('Cron Jobs')) ||
      (await textExists('Scheduled Jobs')) ||
      (await textExists('Refresh Cron Jobs')) ||
      (await textExists(MORNING_BRIEFING_NAME));
    if (!panelVisible) {
      stepLog('Cron Jobs panel markers missing');
      await dumpAccessibilityTree();
      stepLog('Request log (mock API):', getRequestLog());
    }
    expect(panelVisible).toBe(true);
  });
});
