import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { dispatchLocalAiMethod } from '../lib/ai/localCoreAiMemory';
import { CORE_RPC_TIMEOUT_MS, CORE_RPC_URL } from '../utils/config';
import { getStoredRpcUrl } from '../utils/configPersistence';
import { sanitizeError } from '../utils/sanitize';

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

const LEGACY_METHOD_ALIASES: Record<string, string> = {
  'openhuman.get_config': 'openhuman.config_get',
  'openhuman.get_runtime_flags': 'openhuman.config_get_runtime_flags',
  'openhuman.set_browser_allow_all': 'openhuman.config_set_browser_allow_all',
  'openhuman.update_browser_settings': 'openhuman.config_update_browser_settings',
  'openhuman.update_memory_settings': 'openhuman.config_update_memory_settings',
  'openhuman.update_model_settings': 'openhuman.config_update_model_settings',
  'openhuman.update_runtime_settings': 'openhuman.config_update_runtime_settings',
  'openhuman.update_screen_intelligence_settings':
    'openhuman.config_update_screen_intelligence_settings',
  'openhuman.workspace_onboarding_flag_exists': 'openhuman.config_workspace_onboarding_flag_exists',
  'openhuman.workspace_onboarding_flag_set': 'openhuman.config_workspace_onboarding_flag_set',
};

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

function normalizeLegacyMethod(method: string): string {
  if (method in LEGACY_METHOD_ALIASES) {
    return LEGACY_METHOD_ALIASES[method];
  }

  if (method.startsWith('openhuman.auth.')) {
    return `openhuman.auth_${method.slice('openhuman.auth.'.length).split('.').join('_')}`;
  }

  if (method.startsWith('openhuman.accessibility_')) {
    return method.replace('openhuman.accessibility_', 'openhuman.screen_intelligence_');
  }

  return method;
}

export async function getCoreRpcUrl(): Promise<string> {
  if (resolvedCoreRpcUrl) {
    return resolvedCoreRpcUrl;
  }

  if (!coreIsTauri()) {
    // Web environment: check for user-configured RPC URL first
    const storedUrl = getStoredRpcUrl();
    if (storedUrl && storedUrl !== CORE_RPC_URL) {
      resolvedCoreRpcUrl = storedUrl;
      return storedUrl;
    }
    resolvedCoreRpcUrl = CORE_RPC_URL;
    return CORE_RPC_URL;
  }

  if (resolvingCoreRpcUrl) {
    return resolvingCoreRpcUrl;
  }

  const resolvePromise: Promise<string> = (async () => {
    try {
      // Tauri: check for user-configured URL first
      const storedUrl = getStoredRpcUrl();
      if (storedUrl && storedUrl !== CORE_RPC_URL) {
        resolvedCoreRpcUrl = storedUrl;
        return storedUrl;
      }

      const url = await invoke<string>('core_rpc_url');
      const trimmed = String(url || '').trim();
      if (!trimmed) {
        // The Tauri command succeeded but returned an empty string. That's
        // almost certainly a shell misconfiguration — prefer the build-time
        // default but make the fallback visible rather than silent.
        coreRpcError('core_rpc_url returned empty; using build-time default', {
          fallback: CORE_RPC_URL,
        });
      }
      resolvedCoreRpcUrl = trimmed || CORE_RPC_URL;
      return resolvedCoreRpcUrl || CORE_RPC_URL;
    } catch (err) {
      // Fallback to a stored override first, then the build-time default.
      // Keep the underlying invoke failure visible so port mismatches and
      // shell misconfiguration are diagnosable in dev logs.
      const storedUrl = getStoredRpcUrl();
      resolvedCoreRpcUrl = storedUrl || CORE_RPC_URL;
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
 * Returns the per-process RPC bearer token written by the core binary to
 * `~/.openhuman/core.token` at startup.  The token is fetched once via a
 * Tauri command and then cached for the lifetime of the frontend process.
 *
 * Returns `null` in non-Tauri environments (e.g. Vitest) where the command
 * is not available so existing tests remain unaffected.
 */
async function getCoreRpcToken(): Promise<string | null> {
  if (didResolveCoreRpcToken) return resolvedCoreRpcToken;
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

  const normalizedMethod = normalizeLegacyMethod(method);
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
