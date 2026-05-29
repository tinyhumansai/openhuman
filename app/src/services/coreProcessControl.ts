/**
 * Thin wrapper around the Tauri `restart_core_process` IPC command.
 *
 * Surfaced via the Home blocking screen's "Restart Core" button (#1527) so
 * the user has a one-click recovery when the local sidecar has crashed or
 * is stuck. Outside Tauri (web preview / Vitest harness) this is a no-op
 * that returns a friendly error string.
 */
// `safeInvoke` (in place of `@tauri-apps/api/core::invoke`) converts the CEF
// `window.ipc.postMessage` synchronous throw — Sentry TAURI-REACT-7 /
// TAURI-REACT-6 — into a rejected Promise so the caller's `await` /
// `.catch(...)` can handle it instead of bubbling as an unhandled rejection.
import { safeInvoke as invoke, isTauri } from '../utils/tauriCommands/common';
import { clearCoreRpcTokenCache } from './coreRpcClient';

export async function restartCoreProcess(): Promise<void> {
  if (!isTauri()) {
    throw new Error('Restart Core is only available in the desktop app.');
  }
  await invoke('restart_core_process');
  // The Tauri shell mints a fresh `OPENHUMAN_CORE_TOKEN` for the new core
  // process. Drop the cached bearer so token-bearing long-lived consumers
  // (e.g. webhook SSE, see #1922) reconnect with the new value.
  clearCoreRpcTokenCache();
}
