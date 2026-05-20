// @ts-nocheck
/**
 * Tauri IPC bridge spec — proves the renderer can reach the in-process
 * Rust shell and (via `core_rpc_relay`) the embedded core JSON-RPC server.
 *
 * Two layers are checked end-to-end:
 *
 *   1. **Shell commands** (`core_rpc_url`, `core_rpc_token`). These return
 *      the per-launch bearer + RPC URL the renderer uses to talk to the
 *      core. If either of these breaks every RPC the app makes is dead in
 *      the water.
 *
 *   2. **Core RPC over the relay**. We hit `openhuman.about_app_list` — a
 *      cheap read-only method that returns the capability catalogue —
 *      through the same `callOpenhumanRpc` helper every product spec uses.
 *      That round-trips renderer → Tauri IPC → relay → core → response.
 *
 * The Tauri commands are invoked via `window.__TAURI_INTERNALS__.invoke`
 * inside `browser.executeAsync(...)` so the call lives inside the WebView,
 * the same way the React app reaches the shell at runtime via the
 * `@tauri-apps/api/core` `invoke()` helper.
 *
 * Note: under the CEF runtime `window.__TAURI__` (the public namespace) is
 * NOT populated. The underlying IPC bridge lives in
 * `window.__TAURI_INTERNALS__`, which `@tauri-apps/api/core` uses
 * internally. The first test therefore probes `__TAURI_INTERNALS__.invoke`
 * rather than `__TAURI__.core.invoke`.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { hasAppChrome } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';

const USER_ID = 'e2e-tauri-commands';

interface TauriResult<T> {
  __ok?: T;
  __error?: string;
}

async function invokeTauri<T = unknown>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<TauriResult<T>> {
  return (await browser.executeAsync(
    (command, payload, done) => {
      // Under the CEF runtime, Tauri exposes the IPC bridge through
      // `__TAURI_INTERNALS__` (not the public `__TAURI__` namespace which
      // CEF does not populate). `@tauri-apps/api/core`'s `invoke()` helper
      // reads `__TAURI_INTERNALS__.invoke` internally — so this matches the
      // exact transport path the product code uses.
      const internals = (window as any).__TAURI_INTERNALS__;
      if (typeof internals?.invoke !== 'function') {
        done({ __error: 'window.__TAURI_INTERNALS__.invoke not available' });
        return;
      }
      internals
        .invoke(command, payload)
        .then((result: unknown) => done({ __ok: result }))
        .catch((err: unknown) =>
          done({ __error: err instanceof Error ? err.message : String(err) })
        );
    },
    cmd,
    args
  )) as TauriResult<T>;
}

describe('Tauri commands', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await waitForApp();
    await resetApp(USER_ID);
  });

  it('app chrome is visible', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('can take a screenshot (driver bridge is healthy)', async () => {
    const screenshot = await browser.takeScreenshot();
    expect(screenshot).toBeTruthy();
    expect(screenshot.length).toBeGreaterThan(100);
  });

  it('exposes __TAURI_INTERNALS__.invoke to the renderer (CEF IPC bridge)', async () => {
    // Under the CEF runtime `window.__TAURI__` (the public Tauri JS API
    // namespace) is not populated. The underlying IPC bridge used by
    // `@tauri-apps/api/core`'s `invoke()` lives in
    // `window.__TAURI_INTERNALS__.invoke`. Asserting on `__TAURI_INTERNALS__`
    // matches what the product code actually calls at runtime.
    const present = await browser.execute(
      () => typeof (window as any).__TAURI_INTERNALS__?.invoke === 'function'
    );
    expect(present).toBe(true);
  });

  it('core_rpc_url returns a 127.0.0.1 RPC endpoint', async () => {
    const result = await invokeTauri<string>('core_rpc_url');
    expect(result.__error).toBeUndefined();
    expect(String(result.__ok)).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/rpc$/);
  });

  it('core_rpc_token returns a per-launch bearer', async () => {
    const result = await invokeTauri<string>('core_rpc_token');
    expect(result.__error).toBeUndefined();
    const token = String(result.__ok);
    // Hex-encoded random bytes — well over 16 chars in practice.
    expect(token.length).toBeGreaterThanOrEqual(16);
    expect(token).toMatch(/^[A-Za-z0-9]+$/);
  });

  it('round-trips an RPC through the relay (openhuman.about_app_list)', async () => {
    const res = await callOpenhumanRpc('openhuman.about_app_list', {});
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    // RpcOutcome with single_log returns {result: Capability[], logs: [string]}.
    // The outer `result` field (from JSON-RPC) holds that envelope.
    const capabilities = (res.result as { result?: unknown[] })?.result ?? res.result;
    expect(Array.isArray(capabilities)).toBe(true);
    expect((capabilities as unknown[]).length).toBeGreaterThan(0);
  });
});
