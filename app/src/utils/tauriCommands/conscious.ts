/**
 * Conscious loop commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { isTauri } from './common';

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
