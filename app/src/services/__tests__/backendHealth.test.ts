import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { BACKEND_HEALTH_TIMEOUT_MS, checkBackendHealthy } from '../backendHealth';
import { getBackendUrl } from '../backendUrl';

// Local explicit mock — overrides the global one in app/src/test/setup.ts so
// this suite controls `getBackendUrl()` behavior end-to-end (including the
// rejection case for `resolve-failure`) without depending on the global
// mock's resolved value.
vi.mock('../backendUrl', () => ({ getBackendUrl: vi.fn() }));

const mockedGetBackendUrl = vi.mocked(getBackendUrl);

function makeResponse(status: number): Response {
  return new Response(JSON.stringify({ status: 'ok' }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('checkBackendHealthy', () => {
  beforeEach(() => {
    mockedGetBackendUrl.mockResolvedValue('https://backend.test');
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('returns healthy when GET /health responds 200', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(makeResponse(200));

    const result = await checkBackendHealthy({ fetchImpl });

    expect(result.healthy).toBe(true);
    if (result.healthy) {
      expect(result.status).toBe(200);
      expect(result.latencyMs).toBeGreaterThanOrEqual(0);
    }
    expect(fetchImpl).toHaveBeenCalledWith(
      'https://backend.test/health',
      expect.objectContaining({ method: 'GET', cache: 'no-store', credentials: 'omit' })
    );
  });

  it('treats 5xx upstream errors (Cloudflare 504) as unhealthy with reason http-5xx', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(makeResponse(504));

    const result = await checkBackendHealthy({ fetchImpl });

    expect(result.healthy).toBe(false);
    if (!result.healthy) {
      expect(result.reason).toBe('http-5xx');
      expect(result.status).toBe(504);
    }
  });

  it('treats 4xx as healthy — the backend is at least reachable', async () => {
    // /health may not exist in every environment; a 404 still proves the
    // edge + origin are answering. Only 5xx + network failures count as
    // "service down" for the OAuth banner.
    const fetchImpl = vi.fn().mockResolvedValue(makeResponse(404));

    const result = await checkBackendHealthy({ fetchImpl });

    expect(result.healthy).toBe(true);
  });

  it('returns reason=timeout when the fetch aborts after the timeout budget', async () => {
    const fetchImpl: typeof fetch = vi.fn((_input, init?: RequestInit) => {
      return new Promise<Response>((_resolve, reject) => {
        // Mimic real fetch: reject with an AbortError when the caller's
        // AbortSignal fires.
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    });

    const result = await checkBackendHealthy({ fetchImpl, timeoutMs: 20 });

    expect(result.healthy).toBe(false);
    if (!result.healthy) {
      expect(result.reason).toBe('timeout');
    }
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('returns reason=network when fetch rejects (DNS / offline / CORS / TLS)', async () => {
    const fetchImpl = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));

    const result = await checkBackendHealthy({ fetchImpl });

    expect(result.healthy).toBe(false);
    if (!result.healthy) {
      expect(result.reason).toBe('network');
    }
  });

  it('returns reason=resolve-failure when getBackendUrl throws', async () => {
    mockedGetBackendUrl.mockRejectedValueOnce(new Error('Core returned an empty backend URL'));
    const fetchImpl = vi.fn();

    const result = await checkBackendHealthy({ fetchImpl });

    expect(result.healthy).toBe(false);
    if (!result.healthy) {
      expect(result.reason).toBe('resolve-failure');
    }
    expect(fetchImpl).not.toHaveBeenCalled();
  });

  it('exposes a default timeout budget that is short enough for a click handler', () => {
    // Pre-flight runs inline on OAuth button click — a multi-second default
    // would feel like a stuck UI when the backend is slow. Locking the
    // public default to <= 6s catches drift.
    expect(BACKEND_HEALTH_TIMEOUT_MS).toBeLessThanOrEqual(6_000);
  });
});
