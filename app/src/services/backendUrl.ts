import { BACKEND_URL } from '../utils/config';
import { isTauri as coreIsTauri } from '../utils/tauriCommands/common';
import { callCoreRpc } from './coreRpcClient';

let resolvedBackendUrl: string | null = null;
let resolvingBackendUrl: Promise<string> | null = null;
/**
 * Monotonically-increasing generation counter. Incremented on every
 * `clearBackendUrlCache()` call so that any in-flight `getBackendUrl()`
 * resolution started before the clear does not repopulate the cache with a
 * stale value after the user changes their RPC endpoint.
 */
let backendUrlGeneration = 0;

/**
 * Invalidate the cached backend URL so the next call to getBackendUrl()
 * re-derives from the core RPC (Tauri) or web fallback.
 * Call this after the user saves a new RPC URL preference so the backend
 * URL is recomputed from the updated core endpoint.
 */
export function clearBackendUrlCache(): void {
  backendUrlGeneration += 1;
  resolvedBackendUrl = null;
  resolvingBackendUrl = null;
}

function normalizeBaseUrl(url: string): string {
  return url.trim().replace(/\/+$/, '');
}

function webFallbackBackendUrl(): string {
  const fromVite = typeof BACKEND_URL === 'string' ? BACKEND_URL.trim() : '';
  if (fromVite) {
    return normalizeBaseUrl(fromVite);
  }
  if (typeof window !== 'undefined' && window.location?.origin) {
    return normalizeBaseUrl(window.location.origin);
  }
  return 'http://127.0.0.1:3000';
}

export async function getBackendUrl(): Promise<string> {
  if (resolvedBackendUrl) {
    return resolvedBackendUrl;
  }

  if (!coreIsTauri()) {
    resolvedBackendUrl = webFallbackBackendUrl();
    return resolvedBackendUrl;
  }

  if (resolvingBackendUrl) {
    return resolvingBackendUrl;
  }

  const generation = backendUrlGeneration;
  resolvingBackendUrl = (async () => {
    const response = await callCoreRpc<{ api_url?: string; apiUrl?: string }>({
      method: 'openhuman.config_resolve_api_url',
    });
    const resolved = String(response.api_url ?? response.apiUrl ?? '').trim();
    if (!resolved) {
      throw new Error('Core returned an empty backend URL');
    }
    const normalized = normalizeBaseUrl(resolved);
    if (generation === backendUrlGeneration) {
      resolvedBackendUrl = normalized;
    }
    return normalized;
  })().finally(() => {
    if (generation === backendUrlGeneration) {
      resolvingBackendUrl = null;
    }
  });

  return resolvingBackendUrl;
}
