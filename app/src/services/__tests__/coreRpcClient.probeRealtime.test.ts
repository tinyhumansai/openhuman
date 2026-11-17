import { afterEach, describe, expect, test, vi } from 'vitest';

import { probeCoreRealtime } from '../coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => false) }));

function stubFetch(status: number, body: unknown) {
  const fetchMock = vi.fn(
    async () => new Response(JSON.stringify(body), { status }) as unknown as Response
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

describe('probeCoreRealtime', () => {
  afterEach(() => vi.unstubAllGlobals());

  test('reports disabled only for the core 503 socketio_disabled body', async () => {
    const fetchMock = stubFetch(503, { ok: false, error: 'socketio_disabled' });
    await expect(probeCoreRealtime('https://core.example.com/rpc')).resolves.toBe('disabled');
    expect(fetchMock.mock.calls[0]).toEqual([
      'https://core.example.com/socket.io/?EIO=4&transport=polling',
      expect.objectContaining({ method: 'GET' }),
    ]);
  });

  test('an unrelated 503 (for example a proxy) is inconclusive', async () => {
    stubFetch(503, { error: 'bad_gateway' });
    await expect(probeCoreRealtime('https://core.example.com/rpc')).resolves.toBe('unknown');
  });

  test('a 200 handshake means realtime is on', async () => {
    stubFetch(200, {});
    await expect(probeCoreRealtime('https://core.example.com/rpc')).resolves.toBe('ok');
  });

  test('a network failure is inconclusive, not an error', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('network down')));
    await expect(probeCoreRealtime('https://core.example.com/rpc')).resolves.toBe('unknown');
  });
});
