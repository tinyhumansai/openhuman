import { afterEach, describe, expect, it, vi } from 'vitest';

import { LocalTransport } from './LocalTransport';

const URL = 'http://127.0.0.1:7788/rpc';

function mockFetchOnce(
  body: unknown,
  init: { ok?: boolean; status?: number; statusText?: string } = {}
) {
  const fetchMock = vi
    .fn()
    .mockResolvedValue({
      ok: init.ok ?? true,
      status: init.status ?? 200,
      statusText: init.statusText ?? 'OK',
      json: async () => body,
      text: async () => (typeof body === 'string' ? body : JSON.stringify(body)),
    });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const getUrl = () => Promise.resolve(URL);
const getToken =
  (token: string | null = 'tok-xyz') =>
  () =>
    Promise.resolve(token);

describe('LocalTransport', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('resolves URL+token lazily and attaches Authorization when token present', async () => {
    const fetchMock = mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'ok' });
    const t = new LocalTransport(getUrl, getToken('tok-xyz'));

    await t.call('openhuman.ping', { a: 1 });

    expect(fetchMock).toHaveBeenCalledWith(URL, expect.anything());
    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer tok-xyz');
    expect(headers['Content-Type']).toBe('application/json');
  });

  it('omits Authorization when token getter returns null', async () => {
    const fetchMock = mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'ok' });
    const t = new LocalTransport(getUrl, getToken(null));

    await t.call('openhuman.ping', {});

    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers).not.toHaveProperty('Authorization');
  });

  it('throws on HTTP failure', async () => {
    mockFetchOnce('upstream timeout', { ok: false, status: 504, statusText: 'Gateway Timeout' });
    const t = new LocalTransport(getUrl, getToken());
    await expect(t.call('openhuman.ping', {})).rejects.toThrow(/HTTP 504: upstream timeout/);
  });

  it('surfaces JSON-RPC error.message', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, error: { code: 1, message: 'local rpc broke' } });
    const t = new LocalTransport(getUrl, getToken());
    await expect(t.call('openhuman.fail', {})).rejects.toThrow('local rpc broke');
  });

  it('throws when result key is missing', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1 });
    const t = new LocalTransport(getUrl, getToken());
    await expect(t.call('openhuman.ping', {})).rejects.toThrow('response missing result');
  });

  it('merges a caller-supplied abort signal with the internal timeout', async () => {
    const fetchMock = vi.fn().mockImplementation(
      (_url: string, init?: RequestInit) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            const e = new Error('aborted');
            e.name = 'AbortError';
            reject(e);
          });
        })
    );
    vi.stubGlobal('fetch', fetchMock);

    const t = new LocalTransport(getUrl, getToken(), 30);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow(/timed out after 30ms/);
  });

  it('isHealthy + stream + close', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'pong' });
    const t = new LocalTransport(getUrl, getToken());
    await expect(t.isHealthy()).resolves.toBe(true);

    mockFetchOnce({ jsonrpc: '2.0', id: 2, result: 'v' });
    const yielded: string[] = [];
    for await (const v of t.stream<string>('openhuman.value', {})) yielded.push(v);
    expect(yielded).toEqual(['v']);

    await expect(t.close()).resolves.toBeUndefined();
  });

  it('isHealthy returns false when fetch rejects', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('ECONNREFUSED')));
    const t = new LocalTransport(getUrl, getToken());
    await expect(t.isHealthy()).resolves.toBe(false);
  });
});
