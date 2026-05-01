import { beforeEach, describe, expect, test, vi } from 'vitest';

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
  return mod.getBackendUrl;
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

    const getBackendUrl = await loadFreshModule();
    expect(await getBackendUrl()).toBe('https://core-derived.example.com');
    expect(hoisted.callCoreRpcMock).toHaveBeenCalledWith({
      method: 'openhuman.config_resolve_api_url',
    });
  });

  test('caches the resolved URL after the first call so the RPC does not refire', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: 'https://core-derived.example.com' });

    const getBackendUrl = await loadFreshModule();
    await getBackendUrl();
    await getBackendUrl();
    expect(hoisted.callCoreRpcMock).toHaveBeenCalledTimes(1);
  });

  test('throws when core returns an empty api_url so callers do not silently use a stale fallback', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ api_url: '' });

    const getBackendUrl = await loadFreshModule();
    await expect(getBackendUrl()).rejects.toThrow(/empty backend URL/i);
  });

  test('accepts the camelCase apiUrl alias for forward compatibility with future RPC shape', async () => {
    hoisted.isTauriMock.mockReturnValue(true);
    hoisted.callCoreRpcMock.mockResolvedValue({ apiUrl: 'https://core-derived.example.com' });

    const getBackendUrl = await loadFreshModule();
    expect(await getBackendUrl()).toBe('https://core-derived.example.com');
  });
});
