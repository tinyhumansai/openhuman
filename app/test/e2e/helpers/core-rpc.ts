/**
 * Core JSON-RPC for E2E: WebView execute on tauri-driver (Linux), Node fetch on Appium Mac2.
 */
import { callOpenhumanRpcNode } from './core-rpc-node';
import type { RpcCallResult } from './core-rpc-webview';
import { callOpenhumanRpcWebView } from './core-rpc-webview';
import { supportsExecuteScript } from './platform';

export type { RpcCallResult };

export async function callOpenhumanRpc<T = unknown>(
  method: string,
  params: Record<string, unknown> = {}
): Promise<RpcCallResult<T>> {
  if (supportsExecuteScript()) {
    return callOpenhumanRpcWebView<T>(method, params);
  }
  return callOpenhumanRpcNode<T>(method, params);
}
