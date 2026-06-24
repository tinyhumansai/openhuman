/**
 * Regression tests for #3865 — connecting to a self-hosted runtime on a LAN IP
 * from the macOS desktop app.
 *
 * The desktop webview origin is the secure `tauri://localhost` context, so a
 * direct `fetch()` to a non-loopback cleartext-http runtime (e.g.
 * `http://192.168.1.74:7788/rpc`) is blocked as mixed content ("Failed to
 * fetch") before the request ever leaves the browser. The fix relays such
 * requests through the Rust host (`relay_http_rpc` Tauri command); loopback and
 * https URLs keep using the direct fetch path.
 */
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { rpcUrlNeedsShellRelay, testCoreRpcConnection } from '../coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock('../../utils/tauriCommands/common', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/tauriCommands/common')>();
  return { ...actual, isTauri: vi.fn(() => true) };
});
vi.mock('../../lib/ai/localCoreAiMemory', () => ({
  dispatchLocalAiMethod: vi.fn(async () => ({ source: 'local-ai' })),
}));

describe('rpcUrlNeedsShellRelay', () => {
  test('non-loopback cleartext-http URLs need the shell relay', () => {
    expect(rpcUrlNeedsShellRelay('http://192.168.1.74:7788/rpc')).toBe(true);
    expect(rpcUrlNeedsShellRelay('http://10.0.0.5:7788/rpc')).toBe(true);
    expect(rpcUrlNeedsShellRelay('http://my-nas:7788/rpc')).toBe(true);
  });

  test('loopback http is fetchable directly (potentially trustworthy)', () => {
    expect(rpcUrlNeedsShellRelay('http://127.0.0.1:7788/rpc')).toBe(false);
    expect(rpcUrlNeedsShellRelay('http://localhost:7788/rpc')).toBe(false);
    expect(rpcUrlNeedsShellRelay('http://[::1]:7788/rpc')).toBe(false);
    expect(rpcUrlNeedsShellRelay('http://app.localhost:7788/rpc')).toBe(false);
  });

  test('https is never mixed content, so no relay needed', () => {
    expect(rpcUrlNeedsShellRelay('https://192.168.1.74:7788/rpc')).toBe(false);
    expect(rpcUrlNeedsShellRelay('https://core.example.com/rpc')).toBe(false);
  });

  test('malformed URLs do not trigger the relay', () => {
    expect(rpcUrlNeedsShellRelay('not a url')).toBe(false);
    expect(rpcUrlNeedsShellRelay('')).toBe(false);
  });
});

describe('testCoreRpcConnection (self-hosted runtime, #3865)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal('fetch', vi.fn());
  });

  test('relays through the Rust host for a non-loopback http runtime', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      status: 200,
      body: '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}',
    });

    const res = await testCoreRpcConnection('http://192.168.1.74:7788/rpc', 'tok123');

    expect(invoke).toHaveBeenCalledWith('relay_http_rpc', {
      url: 'http://192.168.1.74:7788/rpc',
      token: 'tok123',
      body: expect.stringContaining('core.ping'),
    });
    expect(fetch).not.toHaveBeenCalled();
    expect(res.status).toBe(200);
    expect(res.ok).toBe(true);
    const json = (await res.json()) as { result: { ok: boolean } };
    expect(json.result.ok).toBe(true);
  });

  test('normalizes a base URL without /rpc before relaying', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ status: 200, body: '{}' });

    await testCoreRpcConnection('http://192.168.1.74:7788', 'tok123');

    expect(invoke).toHaveBeenCalledWith(
      'relay_http_rpc',
      expect.objectContaining({ url: 'http://192.168.1.74:7788/rpc' })
    );
  });

  test('uses a direct fetch for a loopback runtime (no relay)', async () => {
    vi.mocked(fetch).mockResolvedValueOnce(
      new Response('{"jsonrpc":"2.0","id":1,"result":{"ok":true}}', { status: 200 })
    );

    await testCoreRpcConnection('http://127.0.0.1:7788/rpc', 'tok123');

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalledWith('relay_http_rpc', expect.anything());
  });

  test('surfaces an upstream 401 verbatim through the relay', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ status: 401, body: 'Unauthorized' });

    const res = await testCoreRpcConnection('http://192.168.1.74:7788/rpc', 'bad-token');

    expect(res.status).toBe(401);
    expect(res.ok).toBe(false);
  });
});
