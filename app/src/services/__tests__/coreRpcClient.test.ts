import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { dispatchLocalAiMethod } from '../../lib/ai/localCoreAiMemory';
import { CORE_RPC_TIMEOUT_MS } from '../../utils/config';
import {
  callCoreRpc,
  classifyAuthExpiredReason,
  classifyRpcError,
  CoreRpcError,
  isThreadNotFoundCoreRpcError,
  setActiveCoreTransport,
} from '../coreRpcClient';
import type { CoreTransport } from '../transport/CoreTransport';

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

  test('broadcasts core-rpc-auth-expired on a SESSION_EXPIRED error by default', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 4,
        error: {
          code: -32000,
          message:
            'SESSION_EXPIRED: backend rejected session token on GET /agent-integrations/composio/triggers/available — sign in again to resume',
        },
      }),
    } as Response);

    const listener = vi.fn();
    window.addEventListener('core-rpc-auth-expired', listener);
    try {
      await expect(callCoreRpc({ method: 'openhuman.team_get' })).rejects.toThrow(
        'SESSION_EXPIRED'
      );
    } finally {
      window.removeEventListener('core-rpc-auth-expired', listener);
    }
    expect(listener).toHaveBeenCalledTimes(1);
  });

  test('suppressAuthExpiredEvent skips the global sign-out broadcast but still throws auth_expired', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 5,
        error: {
          code: -32000,
          message:
            'SESSION_EXPIRED: backend rejected session token on GET /agent-integrations/composio/triggers/available — sign in again to resume',
        },
      }),
    } as Response);

    const listener = vi.fn();
    window.addEventListener('core-rpc-auth-expired', listener);
    let caught: unknown;
    try {
      await callCoreRpc({
        method: 'openhuman.composio_list_available_triggers',
        suppressAuthExpiredEvent: true,
      });
    } catch (err) {
      caught = err;
    } finally {
      window.removeEventListener('core-rpc-auth-expired', listener);
    }
    // The error still surfaces (so the panel can render its in-place CTA)…
    expect(caught).toBeInstanceOf(CoreRpcError);
    expect((caught as CoreRpcError).kind).toBe('auth_expired');
    // …but the global teardown event is NOT broadcast.
    expect(listener).not.toHaveBeenCalled();
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
    ['openhuman.update_model_settings', 'openhuman.inference_update_model_settings'],
    ['openhuman.update_runtime_settings', 'openhuman.config_update_runtime_settings'],
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

      const err = await pending.catch(e => e);
      // The timeout path must throw a CoreRpcError pre-classified as
      // `timeout` so the outer catch does not re-wrap a bare `Error` and so
      // Sentry / call-site `.catch()` can branch on `err.kind`. Regression
      // guard for OPENHUMAN-REACT-Z/Y (the bare-Error shape pre-fix).
      expect(err).toBeInstanceOf(CoreRpcError);
      expect((err as CoreRpcError).kind).toBe('timeout');
      expect((err as Error).message).toBe(
        `Core RPC openhuman.threads_list timed out after ${CORE_RPC_TIMEOUT_MS}ms`
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test('honors per-call timeoutMs override instead of the global default (#2156)', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi.mocked(fetch);
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

      const pending = callCoreRpc({ method: 'openhuman.app_state_snapshot', timeoutMs: 60_000 });
      let settled = false;
      pending
        .catch(() => {})
        .finally(() => {
          settled = true;
        });

      // 30s passes — global default would have aborted by now, but the
      // per-call 60s override keeps the request alive. Assert the pending
      // promise is still in flight so an early-abort regression on the
      // override path cannot slip through (CodeRabbit #2179 review).
      await vi.advanceTimersByTimeAsync(31_000);
      expect(settled).toBe(false);

      // Advance to the override boundary — now the abort fires.
      await vi.advanceTimersByTimeAsync(30_000);

      await expect(pending).rejects.toThrow(
        'Core RPC openhuman.app_state_snapshot timed out after 60000ms'
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test('clamps an oversize timeoutMs to the MAX bound (10 minutes)', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi.mocked(fetch);
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

      const pending = callCoreRpc({
        method: 'openhuman.app_state_snapshot',
        // 2 hours — far beyond the 10 minute clamp; should be reduced.
        timeoutMs: 2 * 60 * 60 * 1_000,
      });
      let settled = false;
      pending
        .catch(() => {})
        .finally(() => {
          settled = true;
        });

      const MAX_MS = 10 * 60 * 1_000;
      // 1ms before the clamp boundary: still pending. Guards against an
      // off-by-one where the clamp accidentally lowers the budget further
      // (CodeRabbit #2179 review).
      await vi.advanceTimersByTimeAsync(MAX_MS - 1);
      expect(settled).toBe(false);

      // Cross the clamp boundary — abort fires.
      await vi.advanceTimersByTimeAsync(2);

      await expect(pending).rejects.toThrow(
        `Core RPC openhuman.app_state_snapshot timed out after ${MAX_MS}ms`
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test('falls back to the global default when timeoutMs is undefined', async () => {
    vi.useFakeTimers();
    try {
      const fetchMock = vi.mocked(fetch);
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
      if (cmd === 'core_rpc_endpoint') {
        return { url: 'http://127.0.0.1:7788/rpc', token: 'test-local-token' };
      }
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
      if (cmd === 'core_rpc_endpoint') throw new Error('denied');
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
      if (cmd === 'core_rpc_endpoint') throw new Error('denied');
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
      .mock.calls.filter(([cmd]) => cmd === 'core_rpc_endpoint').length;
    expect(tokenCalls).toBe(1);
    expect(fetch).not.toHaveBeenCalled();
  });

  describe('active transport forwarding (#5820)', () => {
    function fakeTransport(): CoreTransport & { call: ReturnType<typeof vi.fn> } {
      return {
        kind: 'lan-http',
        call: vi.fn().mockResolvedValue({ requested: true }),
        stream: vi.fn(),
        isHealthy: vi.fn().mockResolvedValue(true),
        close: vi.fn().mockResolvedValue(undefined),
      } as unknown as CoreTransport & { call: ReturnType<typeof vi.fn> };
    }

    afterEach(() => {
      setActiveCoreTransport(null);
    });

    test('forwards a caller-supplied per-call budget to the active transport', async () => {
      // Tunnel and cloud requests used to ignore `timeoutMs` entirely, so a
      // memory source sync was cut off at the transport's default while the
      // core kept working.
      const transport = fakeTransport();
      setActiveCoreTransport(transport);

      await callCoreRpc({
        method: 'openhuman.memory_sources_sync',
        params: { source_id: 'src_1' },
        timeoutMs: 600_000,
      });

      expect(transport.call).toHaveBeenCalledWith(
        'openhuman.memory_sources_sync',
        { source_id: 'src_1' },
        { timeoutMs: 600_000 }
      );
    });

    test('clamps the forwarded budget the same way the local path does', async () => {
      const transport = fakeTransport();
      setActiveCoreTransport(transport);

      await callCoreRpc({ method: 'openhuman.memory_sources_sync', timeoutMs: 99_999_999 });

      expect(transport.call).toHaveBeenCalledWith(
        'openhuman.memory_sources_sync',
        {},
        { timeoutMs: 10 * 60 * 1_000 }
      );
    });

    test('leaves the transport on its own default when no budget is given', async () => {
      const transport = fakeTransport();
      setActiveCoreTransport(transport);

      await callCoreRpc({ method: 'openhuman.memory_sources_sync' });

      expect(transport.call).toHaveBeenCalledTimes(1);
      const [method, params, opts] = transport.call.mock.calls[0] as [string, unknown, unknown];
      expect(method).toBe('openhuman.memory_sources_sync');
      expect(params).toEqual({});
      expect(opts).toBeUndefined();
    });
  });

  describe('testCoreRpcConnection', () => {
    test('POSTs a core.ping JSON-RPC envelope to the supplied URL', async () => {
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
        method: 'core.ping',
        params: {},
      });
    });

    test('normalizes a supplied core base URL before probing', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(false);
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      fetchMock.mockResolvedValueOnce({ ok: true, status: 200 } as Response);

      await testCoreRpcConnection('https://example.trycloudflare.com/');

      expect(fetchMock).toHaveBeenCalledTimes(1);
      expect(fetchMock.mock.calls[0][0]).toBe('https://example.trycloudflare.com/rpc');
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
        if (cmd === 'core_rpc_endpoint')
          return { url: 'http://127.0.0.1:7788/rpc', token: 'deadbeef' };
        throw new Error(`unexpected command: ${cmd}`);
      });
      const { testCoreRpcConnection } = await import('../coreRpcClient');
      const fetchMock = vi.mocked(fetch);
      fetchMock.mockResolvedValueOnce({ ok: true, status: 200 } as Response);

      // Trustworthy localhost http stays on the direct fetch path even in
      // Tauri (no shell relay), so the bearer header is attached here. A
      // non-trustworthy LAN host would relay instead — covered separately below.
      await testCoreRpcConnection('http://127.0.0.1:7788/rpc');

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

    test('relays through the Rust host for non-trustworthy http URLs in Tauri (#3865)', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(true);
      const invokeMock = vi.mocked(invoke);
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'core_rpc_endpoint')
          return { url: 'http://192.168.1.50:7788/rpc', token: 'deadbeef' };
        if (cmd === 'relay_http_rpc') {
          return { status: 200, body: '{"jsonrpc":"2.0","id":1,"result":{}}' };
        }
        throw new Error(`unexpected command: ${cmd}`);
      });
      const { testCoreRpcConnection } = await import('../coreRpcClient');

      const response = await testCoreRpcConnection('http://192.168.1.50:7788/rpc');

      // LAN http can't be fetched cross-origin from the secure tauri webview,
      // so it must be relayed through the Rust host carrying the bearer token.
      const relayCall = invokeMock.mock.calls.find(call => call[0] === 'relay_http_rpc');
      expect(relayCall).toBeDefined();
      const relayArgs = relayCall![1] as { url: string; token: string | null; body: string };
      expect(relayArgs.url).toContain('192.168.1.50');
      expect(relayArgs.token).toBe('deadbeef');
      expect(response.status).toBe(200);
    });

    test('rpcUrlNeedsShellRelay flags only non-trustworthy http URLs', async () => {
      vi.resetModules();
      const { rpcUrlNeedsShellRelay } = await import('../coreRpcClient');
      expect(rpcUrlNeedsShellRelay('http://192.168.1.50:7788/rpc')).toBe(true);
      expect(rpcUrlNeedsShellRelay('http://127.0.0.1:7788/rpc')).toBe(false);
      expect(rpcUrlNeedsShellRelay('http://localhost:7788/rpc')).toBe(false);
      expect(rpcUrlNeedsShellRelay('https://example.test:7788/rpc')).toBe(false);
      expect(rpcUrlNeedsShellRelay('not a url')).toBe(false);
    });

    test('rejects with AbortError when the relay signal is already aborted', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(true);
      vi.mocked(invoke).mockImplementation(async (cmd: string) => {
        if (cmd === 'core_rpc_endpoint')
          return { url: 'http://127.0.0.1:7788/rpc', token: 'deadbeef' };
        if (cmd === 'relay_http_rpc') return { status: 200, body: '{}' };
        throw new Error(`unexpected command: ${cmd}`);
      });
      const { testCoreRpcConnection } = await import('../coreRpcClient');

      const controller = new AbortController();
      controller.abort();
      await expect(
        testCoreRpcConnection('http://192.168.1.50:7788/rpc', undefined, {
          signal: controller.signal,
        })
      ).rejects.toThrow(/abort/i);
    });
  });

  describe('callCoreRpc shell relay (#3865)', () => {
    test('relays via the Rust host for a non-trustworthy http core URL in Tauri', async () => {
      vi.resetModules();
      vi.mocked(isTauri).mockReturnValue(true);
      const invokeMock = vi.mocked(invoke);
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'core_rpc_endpoint') {
          return { url: 'http://192.168.1.50:7788/rpc', token: 'deadbeef' };
        }
        if (cmd === 'relay_http_rpc') {
          return { status: 200, body: '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}' };
        }
        throw new Error(`unexpected command: ${cmd}`);
      });
      const fetchMock = vi.mocked(fetch);
      const { callCoreRpc } = await import('../coreRpcClient');

      const result = await callCoreRpc<{ ok: boolean }>({ method: 'openhuman.threads_list' });

      // A LAN http core URL must be relayed through the Rust host (with the
      // call's abort signal), never fetched cross-origin from the webview.
      expect(result).toEqual({ ok: true });
      expect(fetchMock).not.toHaveBeenCalled();
      expect(invokeMock.mock.calls.some(call => call[0] === 'relay_http_rpc')).toBe(true);
    });
  });
});

describe('classifyRpcError', () => {
  test.each([
    ['GET /teams failed (401 Unauthorized): {"success":false}', undefined, 'auth_expired'],
    ['Session expired. Please log in again.', undefined, 'auth_expired'],
    ['some prefix Session expired suffix', undefined, 'auth_expired'],
    [
      'composio unavailable: no backend session token. Sign in first (auth_store_session).',
      undefined,
      'auth_expired',
    ],
    ['no backend session token; run auth_store_session first', undefined, 'auth_expired'],
    ['NO BACKEND SESSION TOKEN', undefined, 'auth_expired'],
    ['HTTP 429 rate-limit exceeded', undefined, 'rate_limited'],
    // #5157 verbatim from Sentry (CORE-RUST-1PY) — the running core does not
    // expose the method. Permanent, so pollers must be able to stop.
    ['unknown method: openhuman.harness_init_status', undefined, 'method_not_found'],
    ['unknown method: openhuman.memory_tree_create_namespace', undefined, 'method_not_found'],
    ['Budget exceeded for current period', undefined, 'budget_exceeded'],
    ['Insufficient budget for request', undefined, 'budget_exceeded'],
    ['error sending request for url', undefined, 'transport'],
    ['client error (Connect) inner: dns', undefined, 'transport'],
    ['operation timed out after 30s', undefined, 'transport'],
    ['ECONNREFUSED 127.0.0.1:7788', undefined, 'transport'],
    // OPENHUMAN-REACT-15/11/10/12 verbatim from Sentry — local AbortController
    // timeout, NOT backend transport. Must classify as `timeout`.
    ['Core RPC openhuman.team_list_teams timed out after 30000ms', undefined, 'timeout'],
    ['Core RPC openhuman.team_list_members timed out after 30000ms', undefined, 'timeout'],
    ['Core RPC openhuman.team_list_invites timed out after 30000ms', undefined, 'timeout'],
    // OPENHUMAN-REACT-Z/Y verbatim (bare-Error shape pre-fix; now CoreRpcError
    // with same message): still kind=timeout under the new classifier.
    ['Core RPC openhuman.app_state_snapshot timed out after 30000ms', undefined, 'timeout'],
    // OPENHUMAN-REACT-13 verbatim — backend-side connect timeout. Body never
    // hits the `timed out after \d+ms` matcher and stays `transport`.
    [
      'backend request GET /teams: error sending request for url (https://api.tinyhumans.ai/teams): client error (Connect): operation timed out',
      undefined,
      'transport',
    ],
    // Issue #2286: downstream provider 401s must NOT clear the user session.
    [
      'Discord API error: Discord list guilds failed (401): Unauthorized',
      undefined,
      'provider_auth',
    ],
    [
      '[composio] list_connections failed: Backend returned 500 Internal Server Error for GET https://api.tinyhumans.ai/agent-integrations/composio/connections: 401 {"error":{"message":"Invalid API key: ak_o1Og5*****","code":10401,"slug":"HTTP_Unauthorized","status":401}}',
      undefined,
      'provider_auth',
    ],
    ['OpenAI API error (401 Unauthorized): invalid api key', undefined, 'provider_auth'],
    ['Anthropic API error (401 Unauthorized): auth error', undefined, 'provider_auth'],
    ['some random message', undefined, 'unknown'],
  ] as const)('%s => %s', (message, status, expected) => {
    expect(classifyRpcError(message, status)).toBe(expected);
  });

  // A 401 on the RPC endpoint is the LOCAL core's bearer gate, not the
  // TinyHumans backend — the backend's own rejections arrive as a JSON-RPC
  // error inside a 200 and are covered by the message cases above. This used
  // to assert `auth_expired`, which paired with a `confirmed` reason and so
  // signed the user out of their account whenever the core's per-launch bearer
  // went stale.
  test('http status 401 is the core bearer gate, not user session expiry', () => {
    expect(classifyRpcError('anything', 401)).toBe('core_auth');
    expect(
      classifyRpcError(
        '{"ok":false,"error":"unauthorized","message":"Missing or invalid Authorization header. Supply \'Authorization: Bearer <token>\'."}',
        401
      )
    ).toBe('core_auth');
  });

  // The backend path must still sign the user out — that IS the server saying
  // the session is gone, and it arrives with no HTTP status because the core
  // returns it as a JSON-RPC error in a 200.
  test('a backend session rejection still classifies as auth_expired', () => {
    expect(
      classifyRpcError('SESSION_EXPIRED: backend rejected session token on GET /teams/me/usage')
    ).toBe('auth_expired');
    expect(classifyRpcError('GET /teams/me/usage failed (401 Unauthorized)')).toBe('auth_expired');
  });

  test('http status 429 wins over message text', () => {
    expect(classifyRpcError('anything', 429)).toBe('rate_limited');
  });

  test('unknown-method match is prefix-anchored, mirroring the Rust strip_prefix', () => {
    // `dispatch::unknown_method_name` classifies with `strip_prefix`, so the
    // frontend anchors identically — a nested/quoted occurrence is not the
    // core telling us *this* call's method is absent.
    expect(classifyRpcError('unknown method: openhuman.harness_init_status')).toBe(
      'method_not_found'
    );
    expect(classifyRpcError('tool failed: unknown method: openhuman.foo_bar')).toBe('unknown');
  });

  test('structured ThreadNotFound data wins over message text', () => {
    expect(
      classifyRpcError('thread thread-123 not found', undefined, { kind: 'ThreadNotFound' })
    ).toBe('thread_not_found');
  });

  test('local AbortController timeout precedence wins over generic transport regex', () => {
    // The `timed out` substring also matches the broader transport arm; the
    // `timed out after \d+ms` arm MUST run first so callers can distinguish
    // a local 30s ceiling from a backend `client error (Connect)` timeout.
    expect(classifyRpcError('Core RPC openhuman.team_list_teams timed out after 30000ms')).toBe(
      'timeout'
    );
  });
});

describe('classifyAuthExpiredReason', () => {
  test.each([
    // Confirmed server-side rejection → safe to sign out immediately.
    // NOTE: there is deliberately no `['anything', 401, 'confirmed']` case any
    // more. A 401 is the local core's bearer gate and no longer reaches here;
    // if one ever did, `unconfirmed` is the safe fallthrough (corroborate
    // before destroying the session) rather than an immediate sign-out.
    ['anything', 401, 'unconfirmed'],
    ['Session expired. Please log in again.', undefined, 'confirmed'],
    ['SESSION_EXPIRED', undefined, 'confirmed'],
    ['GET /teams failed (401 Unauthorized): {"success":false}', undefined, 'confirmed'],
    // "Token not loaded yet" → unconfirmed: fires transiently right after the
    // restart, before the on-disk auth profile is read. Must NOT be treated as
    // a confirmed expiry — `CoreStateProvider` corroborates before logging out.
    ['session jwt required', undefined, 'unconfirmed'],
    ['SESSION JWT REQUIRED', undefined, 'unconfirmed'],
    ['no backend session token; run auth_store_session first', undefined, 'unconfirmed'],
    ['composio unavailable: no backend session token', undefined, 'unconfirmed'],
    // Unknown auth-expired-ish message defaults to the safe (verify) path.
    ['some opaque auth failure', undefined, 'unconfirmed'],
  ] as const)('%s (status=%s) => %s', (message, status, expected) => {
    expect(classifyAuthExpiredReason(message, status)).toBe(expected);
  });
});

describe('coreRpcClient — typed errors + auth-expired event', () => {
  const authExpiredHandler = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('fetch', vi.fn());
    authExpiredHandler.mockReset();
    window.addEventListener('core-rpc-auth-expired', authExpiredHandler);
  });

  afterEach(() => {
    window.removeEventListener('core-rpc-auth-expired', authExpiredHandler);
  });

  test('throws CoreRpcError(kind=auth_expired) on Session expired payload and fires event once', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        error: {
          code: -32000,
          message: 'GET /teams failed (401 Unauthorized): Session expired. Please log in again.',
        },
      }),
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.team_get_usage' })).rejects.toMatchObject({
      name: 'CoreRpcError',
      kind: 'auth_expired',
    });

    expect(authExpiredHandler).toHaveBeenCalledTimes(1);
    const evt = authExpiredHandler.mock.calls[0][0] as CustomEvent<{
      method: string;
      source: string;
    }>;
    expect(evt.type).toBe('core-rpc-auth-expired');
    expect(evt.detail.method).toBe('openhuman.team_get_usage');
    expect(evt.detail.source).toBe('rpc');
  });

  test('throws CoreRpcError(kind=auth_expired) on HTTP 401 (non-ok response) and fires event', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 401,
      statusText: 'Unauthorized',
      text: async () => 'session expired',
    } as Response);

    const err = await callCoreRpc({ method: 'openhuman.threads_list' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('auth_expired');
    expect((err as CoreRpcError).httpStatus).toBe(401);
    expect(authExpiredHandler).toHaveBeenCalledTimes(1);
  });

  test('a 401 from the core refreshes the bearer and retries once, then succeeds', async () => {
    const fetchMock = vi.mocked(fetch);
    // First attempt: the core rejects a stale per-launch bearer.
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 401,
      statusText: 'Unauthorized',
      text: async () =>
        '{"ok":false,"error":"unauthorized","message":"Missing or invalid Authorization header."}',
    } as Response);
    // Retry with a freshly-read bearer succeeds.
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    await expect(callCoreRpc({ method: 'openhuman.threads_list' })).resolves.toEqual({ ok: true });
    expect(fetchMock).toHaveBeenCalledTimes(2);
    // A stale bearer must never be reported as the user's session expiring.
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('a persistent 401 retries exactly once, then surfaces core_auth', async () => {
    const fetchMock = vi.mocked(fetch);
    const reject = () =>
      ({
        ok: false,
        status: 401,
        statusText: 'Unauthorized',
        text: async () => 'unauthorized',
      }) as Response;
    fetchMock.mockResolvedValueOnce(reject());
    fetchMock.mockResolvedValueOnce(reject());

    const err = await callCoreRpc({ method: 'openhuman.threads_list' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('core_auth');
    // Bounded: one refresh attempt, not a loop.
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('classifies budget_exceeded without firing the auth-expired event', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        error: { code: -32000, message: 'Budget exceeded for current period' },
      }),
    } as Response);

    const err = await callCoreRpc({ method: 'openhuman.team_get_usage' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('budget_exceeded');
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('classifies rate_limited without firing the auth-expired event', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 429,
      statusText: 'Too Many Requests',
      text: async () => 'rate-limit exceeded',
    } as Response);

    const err = await callCoreRpc({ method: 'openhuman.team_get_usage' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('rate_limited');
    expect((err as CoreRpcError).httpStatus).toBe(429);
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('network error wrapped as CoreRpcError(kind=transport) with no auth event', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockRejectedValueOnce(
      new Error('error sending request for url (http://x): ECONNREFUSED')
    );

    const err = await callCoreRpc({ method: 'openhuman.threads_list' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('transport');
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('unknown error preserves message', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        error: { code: -32000, message: 'something weird' },
      }),
    } as Response);

    const err = await callCoreRpc({ method: 'openhuman.threads_list' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('unknown');
    expect((err as Error).message).toBe('something weird');
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });

  test('classifies structured ThreadNotFound data without firing the auth-expired event', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        jsonrpc: '2.0',
        id: 1,
        error: {
          code: -32000,
          message: 'thread thread-123 not found',
          data: {
            kind: 'ThreadNotFound',
            thread_id: 'thread-123',
            method: 'openhuman.threads_message_append',
          },
        },
      }),
    } as Response);

    const err = await callCoreRpc({ method: 'openhuman.threads_message_append' }).catch(e => e);
    expect(err).toBeInstanceOf(CoreRpcError);
    expect((err as CoreRpcError).kind).toBe('thread_not_found');
    expect(isThreadNotFoundCoreRpcError(err, 'thread-123')).toBe(true);
    expect(isThreadNotFoundCoreRpcError(err, 'thread-other')).toBe(false);
    expect(authExpiredHandler).not.toHaveBeenCalled();
  });
});

describe('getCoreRpcUrl', () => {
  const normalizeMockRpcUrl = (url: string) => {
    const trimmed = url.replace(/\/+$/, '');
    return trimmed.endsWith('/rpc') ? trimmed : `${trimmed}/rpc`;
  };

  // Each test gets a fresh module so module-level caches are cleared
  beforeEach(() => {
    vi.resetModules();
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(invoke).mockReset();
  });

  test('in web mode returns stored URL when one is stored', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'http://custom-host:9999/rpc',
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(false);

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    expect(url).toBe('http://custom-host:9999/rpc');
  });

  test('in web mode normalizes a stored core base URL', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'https://example.trycloudflare.com/',
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(false);

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    expect(url).toBe('https://example.trycloudflare.com/rpc');
  });

  test('in web mode returns default CORE_RPC_URL when nothing is stored', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => null,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(false);

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    expect(url).toBe('http://127.0.0.1:7788/rpc');
  });

  test('in web mode caches the result — second call does not change the returned value', async () => {
    let callCount = 0;
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => {
        callCount++;
        return null;
      },
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(false);

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const first = await freshGetCoreRpcUrl();
    const second = await freshGetCoreRpcUrl();
    expect(first).toBe(second);
    // peekStoredRpcUrl should only have been called once due to caching
    expect(callCount).toBe(1);
  });

  test('returns fresh value after clearCoreRpcUrlCache()', async () => {
    let storedValue: string | null = null;
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => storedValue,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(false);

    const { getCoreRpcUrl: freshGetCoreRpcUrl, clearCoreRpcUrlCache: freshClear } =
      await import('../coreRpcClient');

    const first = await freshGetCoreRpcUrl();
    expect(first).toBe('http://127.0.0.1:7788/rpc');

    // Change stored value and clear cache
    storedValue = 'http://new-host:8888/rpc';
    freshClear();

    const second = await freshGetCoreRpcUrl();
    expect(second).toBe('http://new-host:8888/rpc');
  });

  test('in Tauri mode calls invoke("core_rpc_endpoint") when no stored URL', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => null,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        return { url: 'http://tauri-resolved:7788/rpc', token: '' };
      }
      throw new Error(`unexpected: ${cmd}`);
    });

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    expect(url).toBe('http://tauri-resolved:7788/rpc');
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('core_rpc_endpoint');
  });

  test('in Tauri mode stored URL takes priority over invoke result', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'http://stored-override:4444/rpc',
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        return { url: 'http://tauri-would-return:7788/rpc', token: '' };
      }
      throw new Error(`unexpected: ${cmd}`);
    });

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    // stored override should win; invoke should NOT have been called
    expect(url).toBe('http://stored-override:4444/rpc');
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });

  test('cloud-picker URL identical to build-time default still wins over local sidecar', async () => {
    // Regression: in the old `storedUrl !== CORE_RPC_URL` check the picker's
    // value was discarded when it coincided with `VITE_OPENHUMAN_CORE_RPC_URL`,
    // silently routing cloud-mode RPC back to the local sidecar.
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'http://127.0.0.1:7788/rpc',
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        throw new Error('should not be consulted when a stored URL exists');
      }
      throw new Error(`unexpected: ${cmd}`);
    });

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    expect(url).toBe('http://127.0.0.1:7788/rpc');
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });

  test('in Tauri mode falls back to CORE_RPC_URL when invoke fails and no stored URL', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => null,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockRejectedValue(new Error('invoke failed'));

    const { getCoreRpcUrl: freshGetCoreRpcUrl } = await import('../coreRpcClient');
    const url = await freshGetCoreRpcUrl();
    // Should fall back to the default
    expect(url).toBe('http://127.0.0.1:7788/rpc');
  });
});

describe('getCoreRpcToken (cloud-mode persistence)', () => {
  const normalizeMockRpcUrl = (url: string) => {
    const trimmed = url.replace(/\/+$/, '');
    return trimmed.endsWith('/rpc') ? trimmed : `${trimmed}/rpc`;
  };

  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    vi.stubGlobal('fetch', vi.fn());
  });

  test('uses stored cloud-mode token before invoking Tauri sidecar token', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'https://core.example.com/rpc',
      getStoredCoreToken: () => 'cloud-token-abc',
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        throw new Error('should not be called when stored token exists');
      }
      throw new Error(`unexpected invoke: ${cmd}`);
    });
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    const { callCoreRpc: freshCallCoreRpc } = await import('../coreRpcClient');
    await freshCallCoreRpc({ method: 'openhuman.ping' });

    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('core_rpc_endpoint', expect.anything());
    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const headers = requestInit.headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer cloud-token-abc');
  });

  test('honours the host-injected notch core token before the cache/store', async () => {
    // The notch / overlay WKWebViews have no Tauri IPC; the Rust host injects
    // the bearer as a global, which must win ahead of the resolution cache.
    (globalThis as { __OPENHUMAN_NOTCH_CORE_TOKEN__?: string }).__OPENHUMAN_NOTCH_CORE_TOKEN__ =
      'notch-bearer-xyz';
    try {
      const { getCoreRpcToken } = await import('../coreRpcClient');
      await expect(getCoreRpcToken()).resolves.toBe('notch-bearer-xyz');
    } finally {
      delete (globalThis as { __OPENHUMAN_NOTCH_CORE_TOKEN__?: string })
        .__OPENHUMAN_NOTCH_CORE_TOKEN__;
    }
  });

  test('clearCoreRpcTokenCache forces a re-resolve on the next call', async () => {
    let storedToken: string | null = 'first-token';
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => 'https://core.example.com/rpc',
      getStoredCoreToken: () => storedToken,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    const { callCoreRpc: freshCallCoreRpc, clearCoreRpcTokenCache } =
      await import('../coreRpcClient');
    await freshCallCoreRpc({ method: 'openhuman.ping' });
    let headers = fetchMock.mock.calls[0][1] as RequestInit;
    expect((headers.headers as Record<string, string>).Authorization).toBe('Bearer first-token');

    // Rotate the stored token; without clearing the cache the old value
    // persists. Clearing it makes the next call re-resolve.
    storedToken = 'second-token';
    clearCoreRpcTokenCache();
    await freshCallCoreRpc({ method: 'openhuman.ping' });
    headers = fetchMock.mock.calls[1][1] as RequestInit;
    expect((headers.headers as Record<string, string>).Authorization).toBe('Bearer second-token');
  });

  test('falls back to Tauri sidecar token when no stored cloud token', async () => {
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => null,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        return { url: 'http://127.0.0.1:7788/rpc', token: 'local-sidecar-token' };
      }
      throw new Error(`unexpected invoke: ${cmd}`);
    });
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ jsonrpc: '2.0', id: 1, result: { ok: true } }),
    } as Response);

    const { callCoreRpc: freshCallCoreRpc } = await import('../coreRpcClient');
    await freshCallCoreRpc({ method: 'openhuman.ping' });

    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const headers = requestInit.headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer local-sidecar-token');
  });

  test('resolves url and token from the same atomic endpoint snapshot (no race)', async () => {
    // The shell answers `core_rpc_url` and `core_rpc_token` as separate
    // commands; if a gateway activation landed between two calls the renderer
    // could pair A's URL with B's bearer. The atomic `core_rpc_endpoint`
    // command returns both halves in one snapshot, so getCoreRpcUrl() and
    // getCoreRpcToken() must share it rather than each re-invoking.
    vi.doMock('../../utils/configPersistence', () => ({
      peekStoredRpcUrl: () => null,
      getStoredCoreToken: () => null,
      normalizeRpcUrl: normalizeMockRpcUrl,
    }));
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'core_rpc_endpoint') {
        return { url: 'http://127.0.0.1:7788/rpc', token: 'consistent-token' };
      }
      throw new Error(`unexpected invoke: ${cmd}`);
    });

    const { getCoreRpcToken: freshGetToken, getCoreRpcUrl: freshGetUrl } =
      await import('../coreRpcClient');

    const [url, token] = await Promise.all([freshGetUrl(), freshGetToken()]);

    expect(url).toBe('http://127.0.0.1:7788/rpc');
    expect(token).toBe('consistent-token');
    const endpointCalls = vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === 'core_rpc_endpoint');
    // One snapshot serves both halves — no independent re-resolution that could
    // pair a stale URL with a fresh bearer.
    expect(endpointCalls.length).toBe(1);
  });
});
