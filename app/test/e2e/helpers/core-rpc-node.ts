/**
 * Core JSON-RPC from the Node/WebdriverIO process (no WebView `execute`).
 * Required for Appium Mac2, which does not support W3C Execute Script in WKWebView.
 *
 * Auth: the in-process core requires a per-launch bearer token that lives only
 * inside the Tauri host. For e2e, debug builds of the Tauri shell write that
 * token to `${tmpdir}/openhuman-e2e-rpc-token` (see
 * `app/src-tauri/src/core_process.rs`). We read it here and attach
 * `Authorization: Bearer …` to every probe + call. Release builds never write
 * the file, so this code degrades to unauthenticated requests (which the core
 * will reject — acceptable since release builds are not the e2e target).
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import type { RpcCallResult } from './core-rpc-webview';

let cachedRpcUrl: string | null = null;

const E2E_TOKEN_FILENAME = 'openhuman-e2e-rpc-token';

function readBearerToken(): string | null {
  const tokenPath = path.join(os.tmpdir(), E2E_TOKEN_FILENAME);
  try {
    const value = fs.readFileSync(tokenPath, 'utf8').trim();
    return value.length > 0 ? value : null;
  } catch {
    return null;
  }
}

function buildHeaders(includeAuth = true): Record<string, string> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (includeAuth) {
    const token = readBearerToken();
    if (token) headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

function normalizeRpcUrl(raw: string): string {
  const t = raw.trim().replace(/\/$/, '');
  return t.endsWith('/rpc') ? t : `${t}/rpc`;
}

function coreHost(): string {
  return (process.env.OPENHUMAN_CORE_HOST || '127.0.0.1').trim() || '127.0.0.1';
}

/** Ports to try when OPENHUMAN_CORE_PORT is unset (matches typical dev sidecar range). */
function defaultPortProbeList(): number[] {
  const raw = process.env.OPENHUMAN_CORE_PORT?.trim();
  if (raw) {
    const p = Number.parseInt(raw, 10);
    if (!Number.isNaN(p) && p > 0 && p < 65536) {
      return [p];
    }
  }
  const ports: number[] = [];
  for (let port = 7788; port <= 7793; port += 1) ports.push(port);
  return ports;
}

async function tryPingRpc(url: string): Promise<boolean> {
  try {
    // Probe without the bearer token: we're iterating candidate ports, and
    // any non-core service that happens to be bound to one of them shouldn't
    // receive our auth credential as a side effect of discovery.
    const res = await fetch(url, {
      method: 'POST',
      headers: buildHeaders(false),
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'core.ping', params: {} }),
    });
    // 401 means "endpoint exists, auth required" — that's a positive match
    // for the core RPC URL; the real call will retry with auth attached.
    if (res.status === 401) return true;
    if (!res.ok) return false;
    const json = (await res.json()) as { error?: { message?: string } };
    return !json.error;
  } catch {
    return false;
  }
}

/**
 * Resolve the sidecar JSON-RPC URL: full `OPENHUMAN_CORE_RPC_URL`, or
 * `OPENHUMAN_CORE_HOST` + `OPENHUMAN_CORE_PORT`, then probe host:port until core.ping succeeds.
 */
export async function resolveCoreRpcUrl(): Promise<string> {
  if (cachedRpcUrl) return cachedRpcUrl;

  const env = process.env.OPENHUMAN_CORE_RPC_URL?.trim();
  if (env) {
    cachedRpcUrl = normalizeRpcUrl(env);
    return cachedRpcUrl;
  }

  const host = coreHost();
  const ports = defaultPortProbeList();
  const deadline = Date.now() + 60_000;

  while (Date.now() < deadline) {
    for (const port of ports) {
      const url = `http://${host}:${port}/rpc`;
      if (await tryPingRpc(url)) {
        cachedRpcUrl = url;
        return url;
      }
    }
    await new Promise(r => setTimeout(r, 1_500));
  }

  throw new Error(
    `Core JSON-RPC not reachable: set OPENHUMAN_CORE_RPC_URL or OPENHUMAN_CORE_HOST/OPENHUMAN_CORE_PORT (tried ${host} ports ${ports.join(', ')})`
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
      headers: buildHeaders(),
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
