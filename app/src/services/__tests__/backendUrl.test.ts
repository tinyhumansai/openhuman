import { beforeEach, describe, expect, test, vi } from 'vitest';

import { BACKEND_URL } from '../../utils/config';

// Global test setup mocks `services/backendUrl` so consumers get a fixed URL
// without RPC. To exercise the real implementation in this file, opt out.
vi.unmock('../backendUrl');

const hoisted = vi.hoisted(() => ({
  isTauriMock: vi.fn(() => false),
  callCoreRpcMock: vi.fn<(args: unknown) => Promise<unknown>>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ isTauri: hoisted.isTauriMock }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: hoisted.callCoreRpcMock }));

async function loadFreshModule() {
  vi.resetModules();
  const mod = await import('../backendUrl');
  return mod;
}

describe('getBackendUrl', () => {
  beforeEach(() => {
    hoisted.isTauriMock.mockReset();
    hoisted.isTauriMock.mockReturnValue(false);
    hoisted.callCoreRpcMock.mockReset();
  });

  test('in Tauri pulls api_url from openhuman.config_resolve_api_url and trims trailing slash', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: 'https://core-derived.example.com/' });

    const { getBackendUrl } = await loadFreshModule();
    expect(await getBackendUrl()).toBe('https://core-derived.example.com');
    expect(hoisted.callCoreRpcMock).toHaveBeenCalledWith({
      method: 'openhuman.config_resolve_api_url',
    });
  });

  test('caches the resolved URL after the first call so the RPC does not refire', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: 'https://core-derived.example.com' });

    const { getBackendUrl } = await loadFreshModule();
    await getBackendUrl();
    await getBackendUrl();
    expect(hoisted.callCoreRpcMock).toHaveBeenCalledTimes(1);
  });

  test('throws when core returns an empty api_url so callers do not silently use a stale fallback', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: '' });

    const { getBackendUrl } = await loadFreshModule();
    await expect(getBackendUrl()).rejects.toThrow(/empty backend URL/i);
  });

  test('accepts the camelCase apiUrl alias for forward compatibility with future RPC shape', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ apiUrl: 'https://core-derived.example.com' });

    const { getBackendUrl } = await loadFreshModule();
    expect(await getBackendUrl()).toBe('https://core-derived.example.com');
  });

  test('clearBackendUrlCache causes the next getBackendUrl() call to re-derive (not use cached value)', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock
      .mockResolvedValueOnce({ api_url: 'https://first-call.example.com' })
      .mockResolvedValueOnce({ api_url: 'https://second-call.example.com' });

    const { getBackendUrl, clearBackendUrlCache } = await loadFreshModule();

    const first = await getBackendUrl();
    expect(first).toBe('https://first-call.example.com');

    clearBackendUrlCache();

    const second = await getBackendUrl();
    expect(second).toBe('https://second-call.example.com');
    expect(hoisted.callCoreRpcMock).toHaveBeenCalledTimes(2);
  });

  test('calling getBackendUrl() twice returns the same value (cache works)', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: 'https://cached.example.com' });

    const { getBackendUrl } = await loadFreshModule();
    const a = await getBackendUrl();
    const b = await getBackendUrl();
    expect(a).toBe(b);
    expect(a).toBe('https://cached.example.com');
  });

  test('after clearBackendUrlCache a second call re-invokes the core RPC', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: 'https://rechecked.example.com' });

    const { getBackendUrl, clearBackendUrlCache } = await loadFreshModule();
    await getBackendUrl();
    clearBackendUrlCache();
    await getBackendUrl();

    expect(hoisted.callCoreRpcMock).toHaveBeenCalledTimes(2);
  });

  test('in non-Tauri mode returns BACKEND_URL directly without calling core RPC', async () => {
    hoisted.isTauriMock.mockReturnValue(false);

    const { getBackendUrl } = await loadFreshModule();
    const url = await getBackendUrl();

    // Should not have attempted an RPC call in non-Tauri mode
    expect(hoisted.callCoreRpcMock).not.toHaveBeenCalled();
    // Should return the configured fallback constant
    expect(url).toBe(BACKEND_URL);
  });

  test('propagates RPC errors in Tauri mode (no silent fallback)', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockRejectedValue(new Error('RPC unavailable'));

    const { getBackendUrl } = await loadFreshModule();
    // The implementation does NOT catch the error — it propagates. Verify the rejection.
    await expect(getBackendUrl()).rejects.toThrow('RPC unavailable');
  });
});
