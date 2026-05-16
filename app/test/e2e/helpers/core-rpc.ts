/**
 * Core JSON-RPC for E2E.
 *
 * Always uses the Node-side HTTP path now. The old WebView path used
 * `await import('@tauri-apps/api/core')` inside `browser.execute` to fetch
 * the RPC URL — that only worked under tauri-driver because Tauri injects
 * an import map into the renderer. Under the unified Appium Chromium
 * driver (which speaks plain CDP to CEF), the dynamic specifier resolves
 * to nothing and the call fails with "Failed to resolve module specifier
 * '@tauri-apps/api/core'".
 *
 * The Node path probes the core's port range, attaches the per-launch
 * bearer token written to `${tmpdir}/openhuman-e2e-rpc-token`, and does
 * the JSON-RPC over plain HTTP — fully driver-agnostic.
 */
import { callOpenhumanRpcNode } from './core-rpc-node';
import type { RpcCallResult } from './core-rpc-webview';

export type { RpcCallResult };

export async function callOpenhumanRpc<T = unknown>(
  method: string,
  params: Record<string, unknown> = {}
): Promise<RpcCallResult<T>> {
  return callOpenhumanRpcNode<T>(method, params);
}
