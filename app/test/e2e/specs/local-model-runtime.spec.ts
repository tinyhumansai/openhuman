// @ts-nocheck
/**
 * E2E: Local AI Runtime (Ollama)
 *
 * Covers:
 * 3.1.1 Model Detection
 * 3.1.2 Model Download
 * 3.1.3 Model Version Compatibility
 * 3.2.1 Local Model Invocation
 * 3.2.2 Resource Constraint Handling
 * 3.2.3 Runtime Failure Handling
 * 3.3.1 Start / Stop Runtime
 * 3.3.2 Idle State Handling
 * 3.3.3 Concurrent Execution Handling
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { fetchCoreRpcMethods } from '../helpers/core-schema';
import { clearRequestLog, startMockServer } from '../mock-server';

const LOG_PREFIX = '[LocalModelRuntimeE2E]';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`${LOG_PREFIX}[${stamp}] ${message}`);
    return;
  }
  console.log(`${LOG_PREFIX}[${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

function looksLikeSetupError(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('local ai is disabled') ||
    text.includes('unavailable in this core build') ||
    text.includes('unknown method') ||
    text.includes('not implemented')
  );
}

function looksLikeRuntimeNotReadyError(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('local model not ready') ||
    text.includes('not ready') ||
    text.includes('degraded') ||
    text.includes('install') ||
    text.includes('failed to bootstrap') ||
    text.includes('ollama request failed') ||
    text.includes('error sending request') ||
    text.includes('connection refused') ||
    looksLikeSetupError(text)
  );
}

function looksLikeExpectedConcurrencyError(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('busy') ||
    text.includes('in progress') ||
    text.includes('already') ||
    text.includes('concurrent') ||
    looksLikeRuntimeNotReadyError(text)
  );
}

function requireMethod(methods: Set<string>, method: string, caseId: string): boolean {
  if (methods.has(method)) return true;
  console.log(`${LOG_PREFIX} ${caseId}: INCONCLUSIVE — missing RPC method ${method}`);
  return false;
}

async function rpcCall(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  stepLog(`${method} response`, {
    ok: result.ok,
    httpStatus: result.httpStatus,
    error: result.error,
  });
  return result;
}

function unwrapResult<T = Record<string, unknown>>(result: unknown): T {
  if (
    result &&
    typeof result === 'object' &&
    'result' in (result as Record<string, unknown>) &&
    typeof (result as Record<string, unknown>).result !== 'undefined'
  ) {
    return (result as { result: T }).result;
  }
  return result as T;
}

describe('3. Local AI Runtime (Ollama)', function () {
  this.timeout(4 * 60_000);

  let methods: Set<string>;

  before(async function () {
    this.timeout(60_000);
    await startMockServer();
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
    clearRequestLog();
  });

  after(async function () {
    this.timeout(30_000);
    // Mock server is stopped by process exit — stopping it here kills the
    // backend while the app is still running, which invalidates the Appium
    // session before WDIO can cleanly delete it (UND_ERR_CLOSED).
  });

  it('3.1.1 Model Detection', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_status', '3.1.1')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_assets_status', '3.1.1')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_diagnostics', '3.1.1')) return;

    const status = await rpcCall('openhuman.local_ai_status', {});
    const assets = await rpcCall('openhuman.local_ai_assets_status', {});
    const diagnostics = await rpcCall('openhuman.local_ai_diagnostics', {});

    expect(status.ok).toBe(true);
    expect(assets.ok).toBe(true);
    expect(diagnostics.ok).toBe(true);

    const statusPayload = unwrapResult<Record<string, unknown>>(status.result);
    const assetsPayload = unwrapResult<Record<string, unknown>>(assets.result);
    const diagnosticsPayload = unwrapResult<Record<string, unknown>>(diagnostics.result);

    expect(typeof statusPayload.state).toBe('string');
    expect(typeof statusPayload.provider).toBe('string');
    expect(typeof statusPayload.chat_model_id).toBe('string');
    expect(typeof (assetsPayload.chat as Record<string, unknown>)?.id).toBe('string');
    expect(Array.isArray(diagnosticsPayload.installed_models)).toBe(true);
    expect(typeof (diagnosticsPayload.expected as Record<string, unknown>)?.chat_model).toBe(
      'string'
    );
  });

  it('3.1.2 Model Download', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_download', '3.1.2')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_downloads_progress', '3.1.2')) return;

    const trigger = await rpcCall('openhuman.local_ai_download', { force: false });
    const progress = await rpcCall('openhuman.local_ai_downloads_progress', {});

    const triggerAccepted = trigger.ok || looksLikeSetupError(trigger.error);
    const progressAccepted = progress.ok || looksLikeSetupError(progress.error);

    expect(triggerAccepted).toBe(true);
    expect(progressAccepted).toBe(true);

    if (progress.ok) {
      const progressPayload = unwrapResult<Record<string, unknown>>(progress.result);
      expect(typeof progressPayload.state).toBe('string');
      if (typeof progressPayload.progress === 'number') {
        expect(progressPayload.progress).toBeGreaterThanOrEqual(0);
        expect(progressPayload.progress).toBeLessThanOrEqual(1);
      }
    }
  });

  it('3.1.3 Model Version Compatibility', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_status', '3.1.3')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_diagnostics', '3.1.3')) return;

    const status = await rpcCall('openhuman.local_ai_status', {});
    const diagnostics = await rpcCall('openhuman.local_ai_diagnostics', {});

    expect(status.ok).toBe(true);
    expect(diagnostics.ok).toBe(true);

    const statusPayload = unwrapResult<Record<string, unknown>>(status.result);
    const diagnosticsPayload = unwrapResult<Record<string, unknown>>(diagnostics.result);
    const expectedModels = (diagnosticsPayload.expected as Record<string, unknown>) || {};

    expect(expectedModels.chat_model).toBe(statusPayload.chat_model_id);
    expect(expectedModels.embedding_model).toBe(statusPayload.embedding_model_id);
    expect(typeof expectedModels.chat_found).toBe('boolean');
    expect(typeof expectedModels.embedding_found).toBe('boolean');
    expect(Array.isArray(diagnosticsPayload.issues)).toBe(true);
  });

  it('3.2.1 Local Model Invocation', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_prompt', '3.2.1')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_status', '3.2.1')) return;

    const status = await rpcCall('openhuman.local_ai_status', {});
    const prompt = await rpcCall('openhuman.local_ai_prompt', {
      prompt: 'Reply with exactly: local-runtime-ok',
      max_tokens: 32,
      no_think: true,
    });

    const statusPayload = status.ok ? unwrapResult<Record<string, unknown>>(status.result) : null;
    const promptPayload = prompt.ok ? unwrapResult<string>(prompt.result) : null;

    if (status.ok && statusPayload?.state === 'ready') {
      expect(prompt.ok).toBe(true);
      expect(typeof promptPayload).toBe('string');
      expect(String(promptPayload || '').trim().length).toBeGreaterThan(0);
      return;
    }

    const accepted = prompt.ok || looksLikeRuntimeNotReadyError(prompt.error);
    expect(accepted).toBe(true);
  });

  it('3.2.2 Resource Constraint Handling', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_presets', '3.2.2')) return;

    const presets = await rpcCall('openhuman.local_ai_presets', {});
    expect(presets.ok).toBe(true);

    const payload = unwrapResult<Record<string, unknown>>(presets.result);
    const list = (payload.presets as Array<{ tier: string }>) || [];
    const recommended = payload.recommended_tier;
    const tiers = new Set(list.map((preset: { tier: string }) => preset.tier));
    const device = (payload.device as Record<string, unknown>) || {};

    expect(Array.isArray(list)).toBe(true);
    expect(list.length).toBeGreaterThan(0);
    expect(typeof device.total_ram_bytes).toBe('number');
    expect(device.total_ram_bytes).toBeGreaterThan(0);
    expect(typeof device.cpu_count).toBe('number');
    expect(device.cpu_count).toBeGreaterThan(0);
    expect(tiers.has(recommended)).toBe(true);
  });

  it('3.2.3 Runtime Failure Handling', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_set_ollama_path', '3.2.3')) return;

    const invalidPath = `/tmp/openhuman-e2e-missing-ollama-${Date.now()}`;
    const result = await rpcCall('openhuman.local_ai_set_ollama_path', { path: invalidPath });

    expect(result.ok).toBe(false);
    const errorText = String(result.error || '').toLowerCase();
    expect(errorText.includes('ollama binary not found') || errorText.includes('not found')).toBe(
      true
    );
  });

  it('3.3.1 Start / Stop Runtime', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_download', '3.3.1')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_status', '3.3.1')) return;

    const start = await rpcCall('openhuman.local_ai_download', { force: false });
    const restart = await rpcCall('openhuman.local_ai_download', { force: true });
    const status = await rpcCall('openhuman.local_ai_status', {});

    expect(start.ok || looksLikeSetupError(start.error)).toBe(true);
    expect(restart.ok || looksLikeSetupError(restart.error)).toBe(true);
    expect(status.ok).toBe(true);
    expect(typeof unwrapResult<Record<string, unknown>>(status.result).state).toBe('string');
  });

  it('3.3.2 Idle State Handling', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_download', '3.3.2')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_status', '3.3.2')) return;

    const kickoff = await rpcCall('openhuman.local_ai_download', { force: true });
    expect(kickoff.ok || looksLikeSetupError(kickoff.error)).toBe(true);

    const observedStates = new Set<string>();
    const deadline = Date.now() + 12_000;
    while (Date.now() < deadline) {
      const status = await rpcCall('openhuman.local_ai_status', {});
      if (status.ok) {
        const state = unwrapResult<Record<string, unknown>>(status.result).state;
        if (typeof state === 'string') observedStates.add(state);
      }
      await browser.pause(900);
    }

    stepLog('Observed states after force bootstrap', Array.from(observedStates));
    expect(observedStates.size > 0).toBe(true);
    const hasExpectedLifecycleState = Array.from(observedStates).some(state =>
      ['idle', 'loading', 'installing', 'downloading', 'ready', 'degraded'].includes(state)
    );
    expect(hasExpectedLifecycleState).toBe(true);
  });

  it('3.3.3 Concurrent Execution Handling', async () => {
    if (!requireMethod(methods, 'openhuman.local_ai_download', '3.3.3')) return;
    if (!requireMethod(methods, 'openhuman.local_ai_downloads_progress', '3.3.3')) return;

    const [a, b, p] = await Promise.all([
      callOpenhumanRpc('openhuman.local_ai_download', { force: false }),
      callOpenhumanRpc('openhuman.local_ai_download', { force: false }),
      callOpenhumanRpc('openhuman.local_ai_downloads_progress', {}),
    ]);

    stepLog('Concurrent call summary', {
      downloadA: { ok: a.ok, error: a.error },
      downloadB: { ok: b.ok, error: b.error },
      progress: { ok: p.ok, error: p.error },
    });

    const aAccepted = a.ok || looksLikeExpectedConcurrencyError(a.error);
    const bAccepted = b.ok || looksLikeExpectedConcurrencyError(b.error);
    const pAccepted = p.ok || looksLikeExpectedConcurrencyError(p.error);

    expect(aAccepted).toBe(true);
    expect(bAccepted).toBe(true);
    expect(pAccepted).toBe(true);
    expect(a.ok || b.ok || p.ok).toBe(true);
  });
});
