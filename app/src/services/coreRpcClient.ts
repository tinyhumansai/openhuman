import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { dispatchLocalAiMethod } from '../lib/ai/localCoreAiMemory';
import { CORE_RPC_TIMEOUT_MS, CORE_RPC_URL } from '../utils/config';
import { getStoredCoreToken, peekStoredRpcUrl } from '../utils/configPersistence';
import { sanitizeError } from '../utils/sanitize';
import { normalizeRpcMethod } from './rpcMethods';

interface CoreRpcRelayRequest {
  method: string;
  params?: unknown;
  serviceManaged?: boolean;
}

interface JsonRpcRequestBody {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params: unknown;
}

interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

interface JsonRpcResponse<T> {
  jsonrpc?: string;
  id?: number | string | null;
  result?: T;
  error?: JsonRpcError;
}

let nextJsonRpcId = 1;
let resolvedCoreRpcUrl: string | null = null;
let resolvingCoreRpcUrl: Promise<string> | null = null;
let resolvedCoreRpcToken: string | null = null;
let didResolveCoreRpcToken = false;
let resolvingCoreRpcToken: Promise<string | null> | null = null;

/**
 * Invalidate the cached core RPC URL so the next call to getCoreRpcUrl()
 * re-resolves from the user-configured or environment-default value.
 * Call this after the user saves a new RPC URL preference.
 */
export function clearCoreRpcUrlCache(): void {
  resolvedCoreRpcUrl = null;
  resolvingCoreRpcUrl = null;
}

/**
 * Invalidate the cached core RPC bearer token so the next call to
 * `getCoreRpcToken()` re-resolves from `getStoredCoreToken()` or the Tauri
 * sidecar. Call after the user saves a new cloud-mode token (or switches
 * mode) so in-flight changes take effect without a full reload.
 */
export function clearCoreRpcTokenCache(): void {
  resolvedCoreRpcToken = null;
  didResolveCoreRpcToken = false;
  resolvingCoreRpcToken = null;
}
const coreRpcLog = debug('core-rpc');
const coreRpcError = debug('core-rpc:error');

function coreRpcErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (err && typeof err === 'object') {
    const maybeMessage = (err as { message?: unknown }).message;
    if (typeof maybeMessage === 'string' && maybeMessage.trim().length > 0) {
      return maybeMessage;
    }
    const maybeError = (err as { error?: unknown }).error;
    if (typeof maybeError === 'string' && maybeError.trim().length > 0) {
      return maybeError;
    }
  }
  return 'Unknown core RPC error';
}

export async function getCoreRpcUrl(): Promise<string> {
  if (resolvedCoreRpcUrl) {
    return resolvedCoreRpcUrl;
  }

  if (!coreIsTauri()) {
    // Web environment: respect any user-stored URL (including one that
    // happens to equal the build-time default). `peekStoredRpcUrl` returns
    // null when nothing is stored, which lets us distinguish "user hasn't
    // chosen yet" from "user chose a value identical to the default".
    const storedUrl = peekStoredRpcUrl();
    resolvedCoreRpcUrl = storedUrl ?? CORE_RPC_URL;
    return resolvedCoreRpcUrl;
  }

  if (resolvingCoreRpcUrl) {
    return resolvingCoreRpcUrl;
  }

  const resolvePromise: Promise<string> = (async () => {
    try {
      // Tauri: any user-stored URL (cloud picker output) wins. Without this
      // a cloud-mode user whose picker URL coincides with the build-time
      // `VITE_OPENHUMAN_CORE_RPC_URL` would be silently routed to whatever
      // `core_rpc_url` returns (typically the local sidecar's
      // `http://127.0.0.1:<port>/rpc`), producing ERR_CONNECTION_REFUSED in
      // cloud mode where no local sidecar is running.
      const storedUrl = peekStoredRpcUrl();
      if (storedUrl) {
        resolvedCoreRpcUrl = storedUrl;
        return storedUrl;
      }

      const url = await invoke<string>('core_rpc_url');
      const trimmed = String(url || '').trim();
      if (!trimmed) {
        coreRpcError('core_rpc_url returned empty; using build-time default', {
          fallback: CORE_RPC_URL,
        });
      }
      resolvedCoreRpcUrl = trimmed || CORE_RPC_URL;
      return resolvedCoreRpcUrl || CORE_RPC_URL;
    } catch (err) {
      // Tauri invoke failed — fall back to stored URL if any, then the
      // build-time default. Keep the underlying invoke failure visible so
      // port mismatches and shell misconfiguration are diagnosable.
      const storedUrl = peekStoredRpcUrl();
      resolvedCoreRpcUrl = storedUrl ?? CORE_RPC_URL;
      coreRpcError('core_rpc_url invoke failed; using fallback RPC URL', {
        fallback: resolvedCoreRpcUrl,
        usedStoredUrl: Boolean(storedUrl),
        error: sanitizeError(err),
      });
      return resolvedCoreRpcUrl;
    } finally {
      resolvingCoreRpcUrl = null;
    }
  })();
  resolvingCoreRpcUrl = resolvePromise;

  return resolvePromise;
}

/**
 * Returns the bearer token for authenticating against the core RPC endpoint.
 *
 * Resolution order:
 *   1. `getStoredCoreToken()` — token entered by the user in the cloud-mode
 *      picker. When set, the desktop is talking to a remote core and the
 *      local-sidecar token would be wrong. Takes priority so cloud mode
 *      always sends the user's own token.
 *   2. Tauri `core_rpc_token` command — the embedded sidecar's per-process
 *      token, written by the core binary to `~/.openhuman/core.token` at
 *      startup. Cached for the lifetime of the frontend process.
 *   3. `null` in non-Tauri environments (e.g. Vitest, web preview) when no
 *      stored token is set so existing tests remain unaffected.
 */
async function getCoreRpcToken(): Promise<string | null> {
  if (didResolveCoreRpcToken) return resolvedCoreRpcToken;

  const storedToken = getStoredCoreToken();
  if (storedToken) {
    resolvedCoreRpcToken = storedToken;
    didResolveCoreRpcToken = true;
    coreRpcLog('core RPC token loaded from cloud-mode persistence');
    return resolvedCoreRpcToken;
  }

  if (!coreIsTauri()) return null;
  if (resolvingCoreRpcToken) return resolvingCoreRpcToken;

  resolvingCoreRpcToken = (async () => {
    try {
      const token = await invoke<string>('core_rpc_token');
      resolvedCoreRpcToken = token?.trim() || null;
      didResolveCoreRpcToken = true;
      coreRpcLog('core RPC token loaded');
      return resolvedCoreRpcToken;
    } catch (err) {
      coreRpcError('failed to load core RPC token', err);
      resolvedCoreRpcToken = null;
      didResolveCoreRpcToken = true;
      return null;
    } finally {
      resolvingCoreRpcToken = null;
    }
  })();

  return resolvingCoreRpcToken;
}

/**
 * Probe an arbitrary core RPC URL with `openhuman.ping`. Used by the
 * Welcome page's "Test Connection" affordance to validate a user-entered
 * RPC URL without going through the cached `getCoreRpcUrl` resolution.
 *
 * Encapsulates the bearer-token + JSON-RPC envelope assembly that would
 * otherwise sit in the calling component, keeping all RPC client behavior
 * inside the service per the project guideline ("Keep Tauri IPC and RPC
 * client calls localized to services … do not scatter `invoke()` or
 * direct RPC calls throughout components").
 *
 * `tokenOverride` lets the cloud-mode picker test a freshly-typed token
 * before it's persisted; without it, falls back to the normal resolution.
 */
export async function testCoreRpcConnection(
  url: string,
  tokenOverride?: string
): Promise<Response> {
  const token = tokenOverride?.trim() || (await getCoreRpcToken());
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return fetch(url, {
    method: 'POST',
    headers,
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'openhuman.ping', params: {} }),
  });
}

export async function getCoreHttpBaseUrl(): Promise<string> {
  const rpcUrl = await getCoreRpcUrl();
  const url = new URL(rpcUrl);
  url.pathname = '';
  url.search = '';
  url.hash = '';
  return url.toString().replace(/\/$/, '');
}

export async function callCoreRpc<T>({
  method,
  params,
  serviceManaged = false, // kept for compatibility; direct frontend RPC does not use relay-level routing.
}: CoreRpcRelayRequest): Promise<T> {
  void serviceManaged;

  if (method.startsWith('ai.')) {
    return dispatchLocalAiMethod(method, (params ?? {}) as Record<string, unknown>) as T;
  }

  const normalizedMethod = normalizeRpcMethod(method);
  const payload: JsonRpcRequestBody = {
    jsonrpc: '2.0',
    id: nextJsonRpcId++,
    method: normalizedMethod,
    params: params ?? {},
  };

  try {
    const [rpcUrl, token] = await Promise.all([getCoreRpcUrl(), getCoreRpcToken()]);
    coreRpcLog('HTTP request', { id: payload.id, method: payload.method });
    if (coreIsTauri() && !token) {
      throw new Error('Core RPC token unavailable in Tauri; local RPC auth cannot be satisfied');
    }

    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }
    // Bound the fetch to CORE_RPC_TIMEOUT_MS. Without this a hung core
    // sidecar will block every caller (and the UI) forever. We use a
    // manual AbortController + setTimeout rather than AbortSignal.timeout()
    // so test fake timers can drive the abort deterministically.
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), CORE_RPC_TIMEOUT_MS);
    let response: Response;
    try {
      response = await fetch(rpcUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (fetchErr) {
      if (controller.signal.aborted) {
        throw new Error(`Core RPC ${payload.method} timed out after ${CORE_RPC_TIMEOUT_MS}ms`);
      }
      throw fetchErr;
    } finally {
      clearTimeout(timeoutId);
    }

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`Core RPC HTTP ${response.status}: ${text || response.statusText}`);
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      coreRpcError('HTTP error response', {
        id: payload.id,
        method: payload.method,
        error: json.error,
      });
      throw new Error(json.error.message || 'Core RPC returned an error');
    }
    if (!Object.prototype.hasOwnProperty.call(json, 'result')) {
      throw new Error('Core RPC response missing result');
    }

    coreRpcLog('HTTP response', { id: payload.id, method: payload.method });
    return json.result as T;
  } catch (err) {
    coreRpcError('Core RPC call failed', sanitizeError(err));
    throw new Error(coreRpcErrorMessage(err));
  }
}
