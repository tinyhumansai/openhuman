import { invoke, isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { dispatchLocalAiMethod } from '../../lib/ai/localCoreAiMemory';
import { CORE_RPC_TIMEOUT_MS } from '../../utils/config';
import type { AccessibilityStatus, CommandResponse } from '../../utils/tauriCommands';
import { callCoreRpc } from '../coreRpcClient';

function sampleAccessibilityStatus(
  overrides: Partial<AccessibilityStatus> = {}
): AccessibilityStatus {
  return {
    platform_supported: true,
    core_process: { pid: 4242, started_at_ms: 1712700000000 },
    permissions: {
      screen_recording: 'denied',
      accessibility: 'granted',
      input_monitoring: 'unknown',
    },
    features: { screen_monitoring: true },
    session: {
      active: false,
      started_at_ms: null,
      expires_at_ms: null,
      remaining_ms: null,
      ttl_secs: 300,
      panic_hotkey: 'Cmd+Shift+.',
      stop_reason: null,
      frames_in_memory: 0,
      last_capture_at_ms: null,
      last_context: null,
      vision_enabled: true,
      vision_state: 'idle',
      vision_queue_depth: 0,
      last_vision_at_ms: null,
      last_vision_summary: null,
    },
    config: {
      enabled: true,
      capture_policy: 'hybrid',
      policy_mode: 'all_except_blacklist',
      baseline_fps: 1,
      vision_enabled: true,
      session_ttl_secs: 300,
      panic_stop_hotkey: 'Cmd+Shift+.',
      autocomplete_enabled: true,
      use_vision_model: true,
      keep_screenshots: false,
      allowlist: [],
      denylist: [],
    },
    denylist: [],
    is_context_blocked: false,
    permission_check_process_path: '/tmp/openhuman-core-aarch64-apple-darwin',
    ...overrides,
  };
}

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => false) }));
vi.mock('../../lib/ai/localCoreAiMemory', () => ({
  dispatchLocalAiMethod: vi.fn(async (_method: string) => ({ source: 'local-ai' })),
}));

describe('coreRpcClient', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('fetch', vi.fn());
  });

  test('normalizes legacy auth methods from dotted to underscored', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.auth.get_state' });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(String(requestInit.body));
    expect(body.method).toBe('openhuman.auth_get_state');
  });

  test('maps accessibility prefix to screen intelligence prefix', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 2, result: { accepted: true } }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.accessibility_status' });

    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const body = JSON.parse(String(requestInit.body));
    expect(body.method).toBe('openhuman.screen_intelligence_status');
  });

  test('fetches accessibility_status CommandResponse with permissions and process path', async () => {
    const fetchMock = vi.mocked(fetch);
    const status = sampleAccessibilityStatus({
      permission_check_process_path:
        '/Users/dev/openhuman/app/src-tauri/binaries/openhuman-core-aarch64-apple-darwin',
    });
    const envelope: CommandResponse<AccessibilityStatus> = {
      result: status,
      logs: ['screen intelligence status fetched'],
    };

    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 99, result: envelope }),
    } as Response);

    const out = await callCoreRpc<CommandResponse<AccessibilityStatus>>({
      method: 'openhuman.accessibility_status',
    });

    expect(out.logs).toContain('screen intelligence status fetched');
    expect(out.result.permissions.screen_recording).toBe('denied');
    expect(out.result.permissions.accessibility).toBe('granted');
    expect(out.result.permissions.input_monitoring).toBe('unknown');
    expect(out.result.core_process?.pid).toBe(4242);
    expect(out.result.permission_check_process_path).toBe(
      '/Users/dev/openhuman/app/src-tauri/binaries/openhuman-core-aarch64-apple-darwin'
    );
  });

  test('throws clean error when JSON-RPC error payload is returned', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 3,
        error: { code: -32000, message: 'boom from core' },
      }),
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.config_get' })).rejects.toThrow('boom from core');
  });

  test('throws on non-ok HTTP response', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 503,
      statusText: 'Service Unavailable',
      text: async () => 'temporarily unavailable',
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.config_get' })).rejects.toThrow(
      'Core RPC HTTP 503: temporarily unavailable'
    );
  });

  test('routes ai methods to local dispatch without HTTP', async () => {
    const localDispatchMock = vi.mocked(dispatchLocalAiMethod);
    localDispatchMock.mockResolvedValueOnce({ state: 'ready' });

    const result = await callCoreRpc<{ state: string }>({ method: 'ai.get_config', params: {} });

    expect(localDispatchMock).toHaveBeenCalledWith('ai.get_config', {});
    expect(fetch).not.toHaveBeenCalled();
    expect(result).toEqual({ state: 'ready' });
  });

  test.each([
    ['openhuman.get_config', 'openhuman.config_get'],
    ['openhuman.get_runtime_flags', 'openhuman.config_get_runtime_flags'],
    ['openhuman.set_browser_allow_all', 'openhuman.config_set_browser_allow_all'],
    ['openhuman.update_browser_settings', 'openhuman.config_update_browser_settings'],
    ['openhuman.update_memory_settings', 'openhuman.config_update_memory_settings'],
    ['openhuman.update_model_settings', 'openhuman.config_update_model_settings'],
    ['openhuman.update_runtime_settings', 'openhuman.config_update_runtime_settings'],
    [
      'openhuman.update_screen_intelligence_settings',
      'openhuman.config_update_screen_intelligence_settings',
    ],
    [
      'openhuman.workspace_onboarding_flag_exists',
      'openhuman.config_workspace_onboarding_flag_exists',
    ],
    ['openhuman.workspace_onboarding_flag_set', 'openhuman.config_workspace_onboarding_flag_set'],
  ])('rewrites legacy alias %s -> %s', async (incoming, expected) => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callCoreRpc({ method: incoming });
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body));
    expect(body.method).toBe(expected);
  });

  test('passes through unknown methods unchanged', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.threads_list' });
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body));
    expect(body.method).toBe('openhuman.threads_list');
  });

  test('defaults params to empty object when omitted', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.threads_list' });
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body));
    expect(body.params).toEqual({});
    expect(body.jsonrpc).toBe('2.0');
    expect(typeof body.id).toBe('number');
  });

  test('passes through provided params verbatim', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    const params = { thread_id: 't-1', nested: { flag: true } };
    await callCoreRpc({ method: 'openhuman.threads_messages_list', params });
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body));
    expect(body.params).toEqual(params);
  });

  test('increments jsonrpc id on sequential calls', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 0, result: {} }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.threads_list' });
    await callCoreRpc({ method: 'openhuman.threads_list' });
    const idA = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body)).id;
    const idB = JSON.parse(String((fetchMock.mock.calls[1][1] as RequestInit).body)).id;
    expect(typeof idA).toBe('number');
    expect(typeof idB).toBe('number');
    expect(idB).toBe(idA + 1);
  });

  test('throws when JSON-RPC response is missing both result and error', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1 }),
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'Core RPC response missing result'
    );
  });

  test('falls back to generic error message when error.message is blank', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, error: { code: -32000, message: '' } }),
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'Core RPC returned an error'
    );
  });

  test('wraps network errors with message propagated through', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockRejectedValueOnce(new Error('ECONNREFUSED sidecar'));

    await expect(callCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'ECONNREFUSED sidecar'
    );
  });

  test('rewrites multi-segment auth methods (auth.sub.segment) to underscore form', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.auth.sub.segment' });
    const body = JSON.parse(String((fetchMock.mock.calls[0][1] as RequestInit).body));
    expect(body.method).toBe('openhuman.auth_sub_segment');
  });

  test('rejects with a timeout error when fetch does not resolve within CORE_RPC_TIMEOUT_MS', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi.mocked(fetch);
      // Simulate a hung core: the fetch never resolves, but we honor the
      // AbortSignal so the client's timeout can tear us down.
      fetchMock.mockImplementationOnce(
        (_url, init) =>
          new Promise<Response>((_resolve, reject) => {
            const signal = (init as RequestInit).signal as AbortSignal | undefined;
            if (!signal) return;
            const onAbort = () => {
              const err = new Error('The operation was aborted');
              err.name = 'AbortError';
              reject(err);
            };
            if (signal.aborted) onAbort();
            else signal.addEventListener('abort', onAbort, { once: true });
          })
      );

      const pending = callCoreRpc({ method: 'openhuman.threads_list' });
      // Swallow the unhandled rejection that would otherwise be raised when
      // advancing timers triggers the abort before the `await expect` below.
      pending.catch(() => {});

      await vi.advanceTimersByTimeAsync(CORE_RPC_TIMEOUT_MS + 1);

      await expect(pending).rejects.toThrow(
        `Core RPC openhuman.threads_list timed out after ${CORE_RPC_TIMEOUT_MS}ms`
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test('does not trigger the timeout path when fetch resolves promptly', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    const result = await callCoreRpc<{ ok: boolean }>({ method: 'openhuman.threads_list' });
    expect(result).toEqual({ ok: true });

    // Signal on the request init must be populated so the timeout path
    // can tear down a real hung call.
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(init.signal).toBeInstanceOf(AbortSignal);
  });

  test('sends content-type json header and POST method', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callCoreRpc({ method: 'openhuman.threads_list' });
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(init.method).toBe('POST');
    const headers = init.headers as Record<string, string>;
    expect(headers['Content-Type']).toBe('application/json');
  });

  test('adds bearer token header in Tauri mode', async () => {
    vi.resetModules();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_url') return 'http://127.0.0.1:7788/rpc';
      if (cmd === 'core_rpc_token') return 'test-local-token';
      throw new Error(`unexpected command: ${cmd}`);
    });
    const { callCoreRpc: callFreshCoreRpc } = await import('../coreRpcClient');

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }),
    } as Response);

    await callFreshCoreRpc({ method: 'openhuman.threads_list' });

    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer test-local-token');
  });

  test('fails closed in Tauri mode when core rpc token is unavailable', async () => {
    vi.resetModules();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_url') return 'http://127.0.0.1:7788/rpc';
      if (cmd === 'core_rpc_token') throw new Error('denied');
      throw new Error(`unexpected command: ${cmd}`);
    });
    const { callCoreRpc: callFreshCoreRpc } = await import('../coreRpcClient');

    await expect(callFreshCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'Core RPC token unavailable in Tauri; local RPC auth cannot be satisfied'
    );
    expect(fetch).not.toHaveBeenCalled();
  });

  test('caches a missing token result after the first Tauri lookup failure', async () => {
    vi.resetModules();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_url') return 'http://127.0.0.1:7788/rpc';
      if (cmd === 'core_rpc_token') throw new Error('denied');
      throw new Error(`unexpected command: ${cmd}`);
    });
    const { callCoreRpc: callFreshCoreRpc } = await import('../coreRpcClient');

    await expect(callFreshCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'Core RPC token unavailable in Tauri; local RPC auth cannot be satisfied'
    );
    await expect(callFreshCoreRpc({ method: 'openhuman.threads_list' })).rejects.toThrow(
      'Core RPC token unavailable in Tauri; local RPC auth cannot be satisfied'
    );

    const tokenCalls = vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === 'core_rpc_token').length;
    expect(tokenCalls).toBe(1);
    expect(fetch).not.toHaveBeenCalled();
  });

  describe('testCoreRpcConnection', () => {
    test('POSTs an openhuman.ping JSON-RPC envelope to the supplied URL', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(false);
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      fetchMock.mockResolvedValueOnce({ ok: true, status: 200 } as Response);

      await testCoreRpcConnection('http://example.test:7788/rpc');

      expect(fetchMock).toHaveBeenCalledTimes(1);
      const [url, init] = fetchMock.mock.calls[0];
      expect(url).toBe('http://example.test:7788/rpc');
      const requestInit = init as RequestInit;
      expect(requestInit.method).toBe('POST');
      expect(JSON.parse(requestInit.body as string)).toMatchObject({
        jsonrpc: '2.0',
        id: 1,
        method: 'openhuman.ping',
        params: {},
      });
    });

    test('omits Authorization header when no bearer token is available (non-Tauri)', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(false);
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      fetchMock.mockResolvedValueOnce({ ok: true, status: 200 } as Response);

      await testCoreRpcConnection('http://example.test:7788/rpc');

      const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
      const headers = requestInit.headers as Record<string, string>;
      expect(headers).toMatchObject({ 'Content-Type': 'application/json' });
      expect(headers).not.toHaveProperty('Authorization');
    });

    test('attaches Authorization: Bearer when the Tauri bearer token resolves', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(true);
      vi.mocked(invoke).mockImplementation(async (cmd: string) => {
        if (cmd === 'core_rpc_token') return 'deadbeef';
        throw new Error(`unexpected command: ${cmd}`);
      });
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      fetchMock.mockResolvedValueOnce({ ok: true, status: 200 } as Response);

      await testCoreRpcConnection('http://example.test:7788/rpc');

      const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
      const headers = requestInit.headers as Record<string, string>;
      expect(headers.Authorization).toBe('Bearer deadbeef');
      expect(headers['Content-Type']).toBe('application/json');
    });

    test('returns the raw fetch Response so callers can inspect status/ok', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(false);
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      const probe = { ok: false, status: 405, statusText: 'Method Not Allowed' } as Response;
      fetchMock.mockResolvedValueOnce(probe);

      const response = await testCoreRpcConnection('http://example.test:7788/rpc');

      expect(response).toBe(probe);
      expect(response.status).toBe(405);
    });
  });
});
