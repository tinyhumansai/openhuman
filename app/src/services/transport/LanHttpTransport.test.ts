import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanHttpTransport } from './LanHttpTransport';

const URL = 'http://192.168.1.10:7788/rpc';

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

describe('LanHttpTransport', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('issues a POST to the configured rpcUrl with JSON-RPC body', async () => {
    const fetchMock = mockFetchOnce({ jsonrpc: '2.0', id: 1, result: { ok: true } });
    const t = new LanHttpTransport(URL);

    const result = await t.call<{ ok: boolean }>('openhuman.ping', { who: 'me' });

    expect(result).toEqual({ ok: true });
    expect(fetchMock).toHaveBeenCalledWith(
      URL,
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({ 'Content-Type': 'application/json' }),
      })
    );
    const body = JSON.parse((fetchMock.mock.calls[0][1] as RequestInit).body as string);
    expect(body).toMatchObject({ jsonrpc: '2.0', method: 'openhuman.ping', params: { who: 'me' } });
    expect(typeof body.id).toBe('number');
  });

  it('throws when the server returns an HTTP error', async () => {
    mockFetchOnce('Server is sad', { ok: false, status: 500, statusText: 'Server Error' });
    const t = new LanHttpTransport(URL);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow(/HTTP 500: Server is sad/);
  });

  it('throws the JSON-RPC error message when present', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'Method not found' } });
    const t = new LanHttpTransport(URL);
    await expect(t.call('openhuman.unknown', {})).rejects.toThrow('Method not found');
  });

  it('throws when result key is missing', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1 });
    const t = new LanHttpTransport(URL);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow('response missing result');
  });

  it('treats AbortController-induced abort as a timeout', async () => {
    vi.useRealTimers();
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

    const t = new LanHttpTransport(URL, 30);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow(/timed out after 30ms/);
  });

  it('isHealthy returns true on a successful ping', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'pong' });
    const t = new LanHttpTransport(URL);
    await expect(t.isHealthy()).resolves.toBe(true);
  });

  it('isHealthy returns false when ping rejects', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('boom')));
    const t = new LanHttpTransport(URL);
    await expect(t.isHealthy()).resolves.toBe(false);
  });

  it('stream yields the single result from call()', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 42 });
    const t = new LanHttpTransport(URL);
    const yielded: number[] = [];
    for await (const v of t.stream<number>('openhuman.value', {})) yielded.push(v);
    expect(yielded).toEqual([42]);
  });

  it('close() is a no-op', async () => {
    const t = new LanHttpTransport(URL);
    await expect(t.close()).resolves.toBeUndefined();
  });
});
