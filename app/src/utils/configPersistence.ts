/**
 * Config persistence utilities for runtime settings.
 *
 * Handles storing/retrieving user preferences like RPC URL using
 * localStorage (web) or Tauri store (desktop).
 */
import { CORE_RPC_URL } from './config';
import { isTauri } from './tauriCommands';

// Storage key for RPC URL preference
const RPC_URL_STORAGE_KEY = 'openhuman_core_rpc_url';

// Default RPC URL — canonical value from config.ts so they can never drift
const DEFAULT_RPC_URL = CORE_RPC_URL;

/**
 * Check if we're running in a Tauri environment.
 * Used to determine storage backend.
 */
export function isTauriEnvironment(): boolean {
  return isTauri();
}

/**
 * Get the stored RPC URL preference.
 *
 * @returns The stored RPC URL or the default if none stored
 */
export function getStoredRpcUrl(): string {
  try {
    const stored = localStorage.getItem(RPC_URL_STORAGE_KEY);
    if (stored && stored.trim().length > 0) {
      return stored.trim();
    }
  } catch {
    // localStorage might be unavailable in some environments
    console.warn('[configPersistence] Unable to access localStorage');
  }
  return DEFAULT_RPC_URL;
}

/**
 * Store the RPC URL preference.
 *
 * @param url - The RPC URL to store
 */
export function storeRpcUrl(url: string): void {
  try {
    if (url && url.trim().length > 0) {
      localStorage.setItem(RPC_URL_STORAGE_KEY, url.trim());
      console.debug('[configPersistence] Stored RPC URL:', { url: url.trim() });
    } else {
      // Allow clearing the stored URL to reset to default
      localStorage.removeItem(RPC_URL_STORAGE_KEY);
      console.debug('[configPersistence] Cleared stored RPC URL');
    }
  } catch {
    console.warn('[configPersistence] Unable to store RPC URL in localStorage');
  }
}

/**
 * Clear the stored RPC URL preference.
 * This will cause the app to use the default RPC URL.
 */
export function clearStoredRpcUrl(): void {
  storeRpcUrl('');
}

/**
 * Validate an RPC URL format.
 *
 * @param url - The URL to validate
 * @returns true if the URL is valid, false otherwise
 */
export function isValidRpcUrl(url: string): boolean {
  if (!url || url.trim().length === 0) {
    return false;
  }

  try {
    const parsed = new URL(url);
    // Must be http or https
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    return false;
  }
}

/**
 * Normalize an RPC URL by trimming whitespace and trailing slashes.
 *
 * @param url - The URL to normalize
 * @returns The normalized URL
 */
export function normalizeRpcUrl(url: string): string {
  return url.trim().replace(/\/+$/, '');
}

/**
 * Get the default RPC URL.
 *
 * @returns The default RPC URL
 */
export function getDefaultRpcUrl(): string {
  return CORE_RPC_URL;
}

/**
 * Build the full RPC endpoint URL from a base URL.
 *
 * @param baseUrl - The base URL (e.g., 'http://127.0.0.1:7788')
 * @returns The full RPC endpoint URL
 */
export function buildRpcEndpoint(baseUrl: string): string {
  const normalized = normalizeRpcUrl(baseUrl);
  return normalized.endsWith('/rpc') ? normalized : `${normalized}/rpc`;
}
