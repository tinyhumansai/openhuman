import { afterEach, describe, expect, it, vi } from 'vitest';

import { CloudHttpTransport } from './CloudHttpTransport';

const URL = 'https://cloud.openhuman.app/rpc';

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

describe('CloudHttpTransport', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('omits Authorization when no bearer token is configured', async () => {
    const fetchMock = mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'ok' });
    const t = new CloudHttpTransport(URL);

    await t.call('openhuman.ping', {});

    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers).not.toHaveProperty('Authorization');
  });

  it('attaches Authorization: Bearer when a token is configured', async () => {
    const fetchMock = mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'ok' });
    const t = new CloudHttpTransport(URL, 'abc.def.ghi');

    await t.call('openhuman.ping', {});

    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer abc.def.ghi');
  });

  it('throws on HTTP failure', async () => {
    mockFetchOnce('nope', { ok: false, status: 502, statusText: 'Bad Gateway' });
    const t = new CloudHttpTransport(URL);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow(/HTTP 502: nope/);
  });

  it('surfaces JSON-RPC error.message', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, error: { code: 1, message: 'cloud rpc broke' } });
    const t = new CloudHttpTransport(URL);
    await expect(t.call('openhuman.fail', {})).rejects.toThrow('cloud rpc broke');
  });

  it('throws when result key is missing', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1 });
    const t = new CloudHttpTransport(URL);
    await expect(t.call('openhuman.ping', {})).rejects.toThrow('response missing result');
  });

  it('isHealthy + stream + close behave like LAN transport', async () => {
    mockFetchOnce({ jsonrpc: '2.0', id: 1, result: 'pong' });
    const t = new CloudHttpTransport(URL);
    await expect(t.isHealthy()).resolves.toBe(true);

    mockFetchOnce({ jsonrpc: '2.0', id: 2, result: 7 });
    const yielded: number[] = [];
    for await (const v of t.stream<number>('openhuman.value', {})) yielded.push(v);
    expect(yielded).toEqual([7]);

    await expect(t.close()).resolves.toBeUndefined();
  });

  it('isHealthy returns false on transport failure', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('connect refused')));
    const t = new CloudHttpTransport(URL);
    await expect(t.isHealthy()).resolves.toBe(false);
  });
});
