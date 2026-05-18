/**
 * Thin wrapper around the Tauri `restart_core_process` IPC command.
 *
 * Surfaced via the Home blocking screen's "Restart Core" button (#1527) so
 * the user has a one-click recovery when the local sidecar has crashed or
 * is stuck. Outside Tauri (web preview / Vitest harness) this is a no-op
 * that returns a friendly error string.
 */
import { invoke } from '@tauri-apps/api/core';

import { isTauri } from '../utils/tauriCommands/common';

export async function restartCoreProcess(): Promise<void> {
  if (!isTauri()) {
    throw new Error('Restart Core is only available in the desktop app.');
  }
  await invoke('restart_core_process');
}
