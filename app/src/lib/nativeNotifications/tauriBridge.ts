import { invoke, isTauri } from '@tauri-apps/api/core';
import debug from 'debug';

const log = debug('native-notifications:bridge');
const errLog = debug('native-notifications:bridge:error');

export type NotificationPermissionState = 'not_tauri' | 'granted' | 'denied' | 'prompt' | 'unknown';

export interface ShowNativeNotificationArgs {
  title: string;
  body: string;
  tag?: string;
}

export interface ShowNativeNotificationResult {
  delivered: boolean;
  reason?: 'not_tauri' | 'send_failed';
  error?: string;
}

function isGrantedState(state: string): boolean {
  return state === 'granted' || state === 'provisional' || state === 'ephemeral';
}

export async function getNotificationPermissionState(options?: {
  requestIfNeeded?: boolean;
}): Promise<NotificationPermissionState> {
  const requestIfNeeded = options?.requestIfNeeded ?? true;
  if (!isTauri()) {
    return 'not_tauri';
  }

  try {
    const grantedRaw = await invoke<boolean | null>('plugin:notification|is_permission_granted');
    if (grantedRaw === true) return 'granted';

    if (!requestIfNeeded) {
      return 'prompt';
    }

    const requestRaw = await invoke<string>('plugin:notification|request_permission');
    const requestState = String(requestRaw ?? 'unknown').toLowerCase();
    if (isGrantedState(requestState)) return 'granted';
    if (requestState === 'denied') return 'denied';
    if (requestState === 'prompt' || requestState === 'default') return 'prompt';

    return 'unknown';
  } catch (err) {
    errLog('getNotificationPermissionState failed: %O', err);
    return 'unknown';
  }
}

/**
 * Request OS notification permission if not already granted.
 * Returns true if permission is (or was just) granted, false otherwise.
 * No-op (returns false) when running outside Tauri.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  const state = await getNotificationPermissionState();
  log('notification permission ensure resolved state=%s', state);
  return state === 'granted';
}

/**
 * Invoke the Tauri shell to show a native OS notification. No-op when the
 * app is running outside Tauri (e.g. Vitest / pure-web dev server).
 */
export async function showNativeNotification(
  args: ShowNativeNotificationArgs
): Promise<ShowNativeNotificationResult> {
  if (!isTauri()) {
    log('not running in tauri, skipping %o', args);
    return { delivered: false, reason: 'not_tauri' };
  }
  try {
    await invoke('plugin:notification|notify', {
      options: { title: args.title, body: args.body, sound: 'default' },
    });
    log('plugin notify success tag=%s', args.tag ?? 'none');
    return { delivered: true };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    errLog('plugin notify failed: %O', err);
    return { delivered: false, reason: 'send_failed', error: message };
  }
}
