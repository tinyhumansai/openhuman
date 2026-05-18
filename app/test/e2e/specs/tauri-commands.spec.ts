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
 * The Tauri commands are invoked via `window.__TAURI__.core.invoke` inside
 * `browser.executeAsync(...)` so the call lives inside the WebView, the
 * same way the React app reaches the shell at runtime. The
 * `window.__TAURI__` direct-access rule from CLAUDE.md applies to product
 * code; E2E specs whose job is to test the bridge itself are the
 * exception.
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
      const tauri = (window as any).__TAURI__;
      if (!tauri?.core?.invoke) {
        done({ __error: 'window.__TAURI__.core.invoke not available' });
        return;
      }
      tauri.core
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
  before(async () => {
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

  it('exposes window.__TAURI__.core.invoke to the renderer', async () => {
    const present = await browser.execute(
      () => typeof (window as any).__TAURI__?.core?.invoke === 'function'
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
    const res = await callOpenhumanRpc<{ capabilities: unknown[] }>('openhuman.about_app_list', {});
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(Array.isArray(res.result.capabilities)).toBe(true);
    expect(res.result.capabilities.length).toBeGreaterThan(0);
  });
});
