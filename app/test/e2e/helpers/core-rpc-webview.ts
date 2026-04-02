// @ts-nocheck
/**
 * Invoke OpenHuman core JSON-RPC from the Tauri WebView (same transport as `callCoreRpc` in the app).
 * Uses `invoke('core_rpc_url')` so the test follows the live sidecar port.
 */

export interface RpcCallResult<T = unknown> {
  ok: boolean;
  httpStatus?: number;
  error?: string;
  result?: T;
}

/** Linux tauri-driver only — Mac2 cannot run this (no WebView execute). Use `callOpenhumanRpc` from core-rpc.ts. */
export async function callOpenhumanRpcWebView<T = unknown>(
  method: string,
  params: Record<string, unknown> = {}
): Promise<RpcCallResult<T>> {
  return browser.execute(
    async (m: string, p: Record<string, unknown>) => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const rpcUrl = await invoke<string>('core_rpc_url');
        const id = Math.floor(Math.random() * 1e9);
        const res = await fetch(rpcUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id, method: m, params: p }),
        });
        const text = await res.text();
        let json: { error?: { message?: string }; result?: unknown };
        try {
          json = JSON.parse(text) as typeof json;
        } catch {
          return {
            ok: false,
            httpStatus: res.status,
            error: `Invalid JSON (${res.status}): ${text.slice(0, 240)}`,
          };
        }
        if (!res.ok) {
          return { ok: false, httpStatus: res.status, error: text.slice(0, 500) };
        }
        if (json.error) {
          return { ok: false, error: json.error.message || JSON.stringify(json.error) };
        }
        return { ok: true, result: json.result as T };
      } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
      }
    },
    method,
    params
  );
}
