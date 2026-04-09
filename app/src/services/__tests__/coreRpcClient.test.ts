import { beforeEach, describe, expect, test, vi } from 'vitest';

import { dispatchLocalAiMethod } from '../../lib/ai/localCoreAiMemory';
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
});
