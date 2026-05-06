import { isTauri as coreIsTauri } from '@tauri-apps/api/core';

import { BACKEND_URL } from '../utils/config';
import { callCoreRpc } from './coreRpcClient';

let resolvedBackendUrl: string | null = null;
let resolvingBackendUrl: Promise<string> | null = null;

/**
 * Invalidate the cached backend URL so the next call to getBackendUrl()
 * re-derives from the core RPC (Tauri) or web fallback.
 * Call this after the user saves a new RPC URL preference so the backend
 * URL is recomputed from the updated core endpoint.
 */
export function clearBackendUrlCache(): void {
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

  resolvingBackendUrl = (async () => {
    const response = await callCoreRpc<{ api_url?: string; apiUrl?: string }>({
      method: 'openhuman.config_resolve_api_url',
    });
    const resolved = String(response.api_url ?? response.apiUrl ?? '').trim();
    if (!resolved) {
      throw new Error('Core returned an empty backend URL');
    }
    resolvedBackendUrl = normalizeBaseUrl(resolved);
    return resolvedBackendUrl;
  })().finally(() => {
    resolvingBackendUrl = null;
  });

  return resolvingBackendUrl;
}
