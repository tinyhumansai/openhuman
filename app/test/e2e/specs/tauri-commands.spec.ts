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
 * the same way the React app reaches the shell at runtime.
 * `window.__TAURI_INTERNALS__` is the low-level IPC channel set up by the
 * Rust side; it is available on all platforms including the custom CEF
 * runtime, whereas `window.__TAURI__` (the higher-level JS namespace) is
 * only injected when the `@tauri-apps/api` init script runs and is not
 * present in the CEF harness.
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
      const invoke = (window as any).__TAURI_INTERNALS__?.invoke;
      if (typeof invoke !== 'function') {
        done({ __error: 'window.__TAURI_INTERNALS__.invoke not available' });
        return;
      }
      invoke(command, payload)
        .then((result: unknown) => done({ __ok: result }))
        .catch((err: unknown) =>
          done({ __error: err instanceof Error ? err.message : String(err) })
        );
    },
    cmd,
    args
  )) as TauriResult<T>;
}

describe('Tauri commands', function () {
  this.timeout(120_000);

  before(async () => {
    try {
      await waitForApp();
      await resetApp(USER_ID);
    } catch (err) {
      console.log('[tauri-commands] setup failed (non-fatal for IPC tests):', err);
    }
  });

  it('app chrome is visible', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('can take a screenshot (driver bridge is healthy)', async () => {
    const screenshot = await browser.takeScreenshot();
    expect(screenshot).toBeTruthy();
    expect(screenshot.length).toBeGreaterThan(100);
  });

  it('exposes window.__TAURI_INTERNALS__.invoke to the renderer', async () => {
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
    // about_app_list uses single_log → result is {result: [...capabilities], logs: [...]}
    const capabilities = (res.result as any)?.result ?? res.result;
    expect(Array.isArray(capabilities)).toBe(true);
    expect((capabilities as unknown[]).length).toBeGreaterThan(0);
  });
});
