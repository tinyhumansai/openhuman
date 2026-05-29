import { getVersion } from '@tauri-apps/api/app';
import { isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const configMock = vi.hoisted(() => ({ isDev: true }));

vi.mock('../../utils/config', () => ({
  APP_VERSION: '0.0.0-test',
  get IS_DEV() {
    return configMock.isDev;
  },
}));

describe('apiClient version headers', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    configMock.isDev = true;
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(callCoreRpc).mockResolvedValue({ result: { version: '0.0.0-core' } });
    vi.stubGlobal('fetch', vi.fn());
  });

  it('adds x-web-version on non-Tauri backend requests', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      headers: new Headers({ 'content-type': 'application/json' }),
      json: async () => ({ success: true }),
    } as Response);

    const { apiClient } = await import('../apiClient');
    await apiClient.get('/version-check', { requireAuth: false });

    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const headers = requestInit.headers as Record<string, string>;
    expect(headers['x-web-version']).toBe('0.0.0-test');
    expect(headers).not.toHaveProperty('x-tauri-version');
  });

  it('adds sanitized x-tauri-version and x-core-version on Tauri backend requests', async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(getVersion).mockResolvedValue(' 1.2.3 (desktop)+build!? ');
    vi.mocked(callCoreRpc).mockResolvedValue({ result: { version: ' 4.5.6 (core)+abc ' } });

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      headers: new Headers({ 'content-type': 'application/json' }),
      json: async () => ({ success: true }),
    } as Response);

    const { apiClient } = await import('../apiClient');
    await apiClient.post('/version-check', { ok: true }, { requireAuth: false });

    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const headers = requestInit.headers as Record<string, string>;
    expect(headers['x-tauri-version']).toBe('1.2.3desktop+build');
    expect(headers['x-core-version']).toBe('4.5.6core+abc');
    expect(headers).not.toHaveProperty('x-web-version');
  });

  it('logs request diagnostics in dev without leaking authorization headers', async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      headers: new Headers({ 'content-type': 'application/json' }),
      json: async () => ({ success: true }),
    } as Response);

    const { apiClient, setStoreForApiClient } = await import('../apiClient');
    setStoreForApiClient(() => 'secret-session-token');

    await apiClient.post('/version-check', { ok: true });

    const requestInit = fetchMock.mock.calls[0][1] as RequestInit;
    const requestHeaders = requestInit.headers as Record<string, string>;
    expect(requestHeaders.Authorization).toBe('Bearer secret-session-token');

    const logMock = vi.mocked(console.log);
    expect(logMock).toHaveBeenCalledWith(
      'request',
      expect.objectContaining({
        method: 'POST',
        headers: expect.not.objectContaining({
          Authorization: expect.any(String),
          authorization: expect.any(String),
        }),
      })
    );
  });

  it('does not log request diagnostics in production', async () => {
    configMock.isDev = false;

    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({
      ok: true,
      headers: new Headers({ 'content-type': 'application/json' }),
      json: async () => ({ success: true }),
    } as Response);

    const { apiClient, setStoreForApiClient } = await import('../apiClient');
    setStoreForApiClient(() => 'secret-session-token');

    await apiClient.get('/version-check');

    expect(console.log).not.toHaveBeenCalledWith('request', expect.anything());
  });

  it('retries tauri version lookup after a transient failure', async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(getVersion)
      .mockRejectedValueOnce(new Error('transient failure'))
      .mockResolvedValueOnce('2.3.4');
    vi.mocked(callCoreRpc).mockResolvedValue({ result: { version: '4.5.6' } });

    const { getClientVersionHeaders } = await import('../clientVersionHeaders');

    await expect(getClientVersionHeaders()).resolves.toEqual({ 'x-core-version': '4.5.6' });
    await expect(getClientVersionHeaders()).resolves.toEqual({
      'x-tauri-version': '2.3.4',
      'x-core-version': '4.5.6',
    });
    expect(getVersion).toHaveBeenCalledTimes(2);
  });
});
