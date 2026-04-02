/**
 * Core JSON-RPC from the Node/WebdriverIO process (no WebView `execute`).
 * Required for Appium Mac2, which does not support W3C Execute Script in WKWebView.
 */
import type { RpcCallResult } from './core-rpc-webview';

let cachedRpcUrl: string | null = null;

function normalizeRpcUrl(raw: string): string {
  const t = raw.trim().replace(/\/$/, '');
  return t.endsWith('/rpc') ? t : `${t}/rpc`;
}

/**
 * Resolve the sidecar JSON-RPC URL: env, then probe 127.0.0.1:7788–7793 until core.ping succeeds.
 */
export async function resolveCoreRpcUrl(): Promise<string> {
  if (cachedRpcUrl) return cachedRpcUrl;

  const env = process.env.OPENHUMAN_CORE_RPC_URL?.trim();
  if (env) {
    cachedRpcUrl = normalizeRpcUrl(env);
    return cachedRpcUrl;
  }

  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    for (let port = 7788; port <= 7793; port += 1) {
      const url = `http://127.0.0.1:${port}/rpc`;
      try {
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'core.ping', params: {} }),
        });
        if (!res.ok) continue;
        const json = (await res.json()) as { error?: { message?: string } };
        if (json.error) continue;
        cachedRpcUrl = url;
        return url;
      } catch {
        /* ECONNREFUSED etc. */
      }
    }
    await new Promise(r => setTimeout(r, 1_500));
  }

  throw new Error(
    'Core JSON-RPC not reachable: set OPENHUMAN_CORE_RPC_URL or ensure the sidecar listens on 127.0.0.1:7788–7793'
  );
}

export async function callOpenhumanRpcNode<T = unknown>(
  method: string,
  params: Record<string, unknown> = {}
): Promise<RpcCallResult<T>> {
  try {
    const rpcUrl = await resolveCoreRpcUrl();
    const id = Math.floor(Math.random() * 1e9);
    const res = await fetch(rpcUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id, method, params }),
    });
    const text = await res.text();
    let json: { error?: { message?: string }; result?: T };
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
    return { ok: true, result: json.result };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}
