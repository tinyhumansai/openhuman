/**
 * Conscious loop commands.
 */
// `safeInvoke` (aliased to `invoke`) replaces bare
// `@tauri-apps/api/core::invoke` so the CEF `window.ipc.postMessage`
// synchronous throw (Sentry TAURI-REACT-7 / TAURI-REACT-6) lands as a
// rejected Promise instead of escaping to `onunhandledrejection`.
import { safeInvoke as invoke, isTauri } from './common';

/**
 * Trigger a conscious loop run manually.
 */
export async function consciousLoopRun(
  authToken: string,
  backendUrl: string,
  model?: string
): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await invoke('conscious_loop_run', { authToken, backendUrl, model });
}
