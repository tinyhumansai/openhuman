/**
 * Common utilities and types for Tauri Commands.
 */
import { isTauri as coreIsTauri } from '@tauri-apps/api/core';
import debug from 'debug';

const log = debug('tauri:ipc-guard');

/**
 * True when the Tauri runtime is present AND the underlying IPC transport is
 * wired. The official `coreIsTauri()` check (which reads `globalThis.isTauri`)
 * is set early by Tauri's webview bootstrap, but on CEF `__TAURI_INTERNALS__`
 * (and the `postMessage` bridge it dispatches through) is injected *after*
 * `on_after_created` fires. An `invoke()` landing in that gap throws
 * `TypeError: Cannot read properties of undefined (reading 'postMessage')`
 * deep inside Tauri's `sendIpcMessage` — see OPENHUMAN-REACT-S / #1472.
 *
 * Callers that gate on `isTauri()` BEFORE invoking should therefore use this
 * function; it returns `false` during the bootstrap gap so the call site
 * takes the non-Tauri branch (skip / fallback) instead of synchronously
 * throwing into a `new Promise` body where the rejection escapes the local
 * try/catch and lands as an unhandled Sentry event.
 */
export const isTauri = (): boolean => {
  if (!coreIsTauri()) return false;
  if (typeof window === 'undefined') return false;
  // Narrow `window` access through a single optional chain so the check is
  // resilient to either `__TAURI_INTERNALS__` being absent or `.invoke`
  // being missing while the rest of the object is partially populated.
  const internals = (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } })
    .__TAURI_INTERNALS__;
  if (typeof internals?.invoke !== 'function') {
    // Bridge-missing branch: distinct from `!coreIsTauri()` (= not in Tauri
    // at all). Logging here makes the CEF bootstrap gap observable in dev
    // and is a no-op in production (debug namespace disabled by default).
    log('isTauri() -> false: IPC bridge not wired (CEF bootstrap gap or non-Tauri)');
    return false;
  }
  return true;
};

export interface CommandResponse<T> {
  result: T;
  logs: string[];
}

export function tauriErrorMessage(err: unknown): string {
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
  return 'Unknown Tauri invoke error';
}

export function parseServiceCliOutput<T>(raw: string): CommandResponse<T> {
  const parsed = JSON.parse(raw) as CommandResponse<T>;
  return parsed;
}
