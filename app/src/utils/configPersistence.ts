/**
 * Config persistence utilities for runtime settings.
 *
 * Handles storing/retrieving user preferences like RPC URL using
 * localStorage (web) or Tauri store (desktop).
 */
import debug from 'debug';

import { CORE_RPC_URL, E2E_DEFAULT_CORE_MODE } from './config';
import { redactRpcUrlForLog } from './redactRpcUrlForLog';
import { isTauri } from './tauriCommands';

export { redactRpcUrlForLog } from './redactRpcUrlForLog';

const log = debug('config-persistence');

// Storage key for RPC URL preference
const RPC_URL_STORAGE_KEY = 'openhuman_core_rpc_url';

// Storage key for cloud-mode bearer token. Pre-login and per-device, parallel
// to the URL key. Held in plain localStorage because the cloud picker runs
// before any user session exists.
const CORE_TOKEN_STORAGE_KEY = 'openhuman_core_rpc_token';

// Storage key for the user-chosen core mode ('local' | 'cloud'). Mirrors the
// redux-persist `coreMode` blob synchronously so reloads (notably the dev-mode
// `window.location.reload()` triggered by `handleIdentityFlip`) can recover
// the chosen mode before redux-persist's async flush completes — without this
// the BootCheckGate flips back to the picker after every reload, producing an
// infinite picker → flip → reload loop in cloud mode.
const CORE_MODE_STORAGE_KEY = 'openhuman_core_mode';

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
      return normalizeRpcUrl(stored);
    }
  } catch {
    // localStorage might be unavailable in some environments
    console.warn('[configPersistence] Unable to access localStorage');
  }
  return DEFAULT_RPC_URL;
}

/**
 * Peek at the stored RPC URL **without** falling back to the build-time
 * default — returns `null` when nothing is stored.
 *
 * Use this to distinguish "user has explicitly chosen a URL" from "nothing
 * stored yet, you're seeing the default". The masked-by-default behavior of
 * `getStoredRpcUrl` makes that distinction impossible: when a user chooses a
 * URL that happens to equal `CORE_RPC_URL` (e.g. the build-time fallback in
 * `app/.env.local` matches their cloud picker input), `getStoredRpcUrl` and
 * the default are indistinguishable, so callers that want to honour the
 * explicit choice unambiguously must read this instead.
 */
export function peekStoredRpcUrl(): string | null {
  try {
    const stored = localStorage.getItem(RPC_URL_STORAGE_KEY);
    if (stored && stored.trim().length > 0) {
      return normalizeRpcUrl(stored);
    }
  } catch {
    console.warn('[configPersistence] Unable to access localStorage');
  }
  return null;
}

/**
 * Store the RPC URL preference.
 *
 * @param url - The RPC URL to store
 */
export function storeRpcUrl(url: string): void {
  try {
    if (url && url.trim().length > 0) {
      const normalized = normalizeRpcUrl(url);
      localStorage.setItem(RPC_URL_STORAGE_KEY, normalized);
      log('Stored RPC URL: %s', redactRpcUrlForLog(normalized));
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
 * Return true when `hostname` is local or private-network address space.
 *
 * This intentionally includes Tailscale/CGNAT (`100.64.0.0/10`): self-hosted
 * cores often run on tailnets where the transport is already encrypted and
 * the HTTP service is not exposed to the public internet.
 */
export function isLocalOrPrivateNetworkHost(hostname: string): boolean {
  const host = hostname
    .trim()
    .replace(/^\[(.*)\]$/, '$1')
    .toLowerCase();
  if (!host) return false;
  if (host === 'localhost' || host.endsWith('.localhost')) return true;
  if (host === '::1') return true;
  if (host.startsWith('fe80:')) return true;
  if (/^f[cd][0-9a-f]{2}:/i.test(host)) return true;

  const match = host.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!match) return false;

  const octets = match.slice(1).map(Number);
  if (octets.some(octet => octet < 0 || octet > 255)) return false;

  const [a, b] = octets;
  return (
    a === 10 ||
    a === 127 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254) ||
    (a === 100 && b >= 64 && b <= 127)
  );
}

/**
 * Cloud cores may use HTTPS on any host. Plain HTTP is accepted only for
 * localhost/private networks, including tailnets, to avoid encouraging
 * bearer-token transport over public plaintext links.
 */
export function isAllowedCloudRpcUrl(url: string): boolean {
  if (!isValidRpcUrl(url)) return false;

  const parsed = new URL(url.trim());
  if (parsed.protocol === 'https:') return true;
  return parsed.protocol === 'http:' && isLocalOrPrivateNetworkHost(parsed.hostname);
}

/**
 * Normalize an RPC URL by trimming whitespace and trailing slashes.
 * When the user provides a core base URL with no path, treat it as the
 * JSON-RPC endpoint base and append `/rpc`.
 *
 * @param url - The URL to normalize
 * @returns The normalized URL
 */
export function normalizeRpcUrl(url: string): string {
  const trimmed = url.trim();
  try {
    // Parse before trimming path slashes so query/hash values such as ?next=/
    // or #/ stay byte-for-byte intact.
    new URL(trimmed);

    const suffixStart = firstUrlSuffixIndex(trimmed);
    const base = suffixStart === -1 ? trimmed : trimmed.slice(0, suffixStart);
    const suffix = suffixStart === -1 ? '' : trimmed.slice(suffixStart);
    const pathStart = base.indexOf('/', base.indexOf('://') + 3);
    const origin = pathStart === -1 ? base : base.slice(0, pathStart);
    const path = pathStart === -1 ? '' : base.slice(pathStart);
    const pathWithoutTrailingSlashes = path.replace(/\/+$/, '');
    const normalizedPath = pathWithoutTrailingSlashes || '/rpc';

    return `${origin}${normalizedPath}${suffix}`;
  } catch {
    // Validation reports malformed URLs. Keep this helper side-effect free.
  }
  return trimmed.replace(/\/+$/, '');
}

function firstUrlSuffixIndex(url: string): number {
  const searchIndex = url.indexOf('?');
  const hashIndex = url.indexOf('#');
  if (searchIndex === -1) return hashIndex;
  if (hashIndex === -1) return searchIndex;
  return Math.min(searchIndex, hashIndex);
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
 * Get the stored cloud-mode bearer token, if any.
 *
 * Returns null when no token is stored (the common case for local-mode users)
 * so the caller can fall back to the local sidecar's per-process token.
 */
export function getStoredCoreToken(): string | null {
  try {
    const stored = localStorage.getItem(CORE_TOKEN_STORAGE_KEY);
    if (stored && stored.trim().length > 0) {
      return stored.trim();
    }
  } catch {
    console.warn('[configPersistence] Unable to access localStorage');
  }
  return null;
}

/**
 * Store the cloud-mode bearer token. An empty string clears the stored value
 * so the caller can flip back to local-sidecar auth without manual cleanup.
 */
export function storeCoreToken(token: string): void {
  try {
    if (token && token.trim().length > 0) {
      localStorage.setItem(CORE_TOKEN_STORAGE_KEY, token.trim());
      console.debug('[configPersistence] Stored core token (cloud mode)');
    } else {
      localStorage.removeItem(CORE_TOKEN_STORAGE_KEY);
      console.debug('[configPersistence] Cleared stored core token');
    }
  } catch {
    console.warn('[configPersistence] Unable to store core token in localStorage');
  }
}

/** Clear the stored cloud-mode bearer token. */
export function clearStoredCoreToken(): void {
  storeCoreToken('');
}

/**
 * Read the synchronous core-mode marker. Returns `null` when nothing has
 * been written yet (first launch, or after `clearStoredCoreMode`).
 */
export function getStoredCoreMode(): 'local' | 'cloud' | null {
  try {
    const stored = localStorage.getItem(CORE_MODE_STORAGE_KEY)?.trim();
    if (stored) {
      if (stored === 'local' || stored === 'cloud') return stored;
      return null;
    }
  } catch {
    console.warn('[configPersistence] Unable to access localStorage');
  }

  if (E2E_DEFAULT_CORE_MODE === 'local') return 'local';
  return null;
}

/** Persist the synchronous core-mode marker. */
export function storeCoreMode(mode: 'local' | 'cloud'): void {
  try {
    localStorage.setItem(CORE_MODE_STORAGE_KEY, mode);
    console.debug('[configPersistence] Stored core mode:', mode);
  } catch {
    console.warn('[configPersistence] Unable to store core mode in localStorage');
  }
}

/** Remove the synchronous core-mode marker (returns the picker to first-launch state). */
export function clearStoredCoreMode(): void {
  try {
    localStorage.removeItem(CORE_MODE_STORAGE_KEY);
  } catch {
    console.warn('[configPersistence] Unable to clear core mode in localStorage');
  }
}
