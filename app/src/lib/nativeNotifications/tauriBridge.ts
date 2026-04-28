import { invoke, isTauri } from '@tauri-apps/api/core';
import debug from 'debug';

const log = debug('native-notifications:bridge');
const errLog = debug('native-notifications:bridge:error');

export interface ShowNativeNotificationArgs {
  title: string;
  body: string;
  tag?: string;
}

/**
 * Request OS notification permission if not already granted.
 * Returns true if permission is (or was just) granted, false otherwise.
 * No-op (returns false) when running outside Tauri.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  if (!isTauri()) {
    log('not running in tauri, skipping permission request');
    return false;
  }
  try {
    const grantedRaw = await invoke<boolean | null>('plugin:notification|is_permission_granted');
    const granted = grantedRaw === true;
    log('notification permission check (plugin): granted=%s raw=%o', granted, grantedRaw);
    if (granted) return true;

    const requestResult = await invoke<string>('plugin:notification|request_permission');
    const requestState = String(requestResult ?? 'unknown').toLowerCase();
    const nowGranted = requestState === 'granted' || requestState === 'provisional';
    log('notification permission request result=%s granted=%s', requestState, nowGranted);
    if (nowGranted) return true;

    // Re-check once after request because some platforms may not return
    // a definitive granted state from request_permission directly.
    const pluginGranted = await invoke<boolean | null>('plugin:notification|is_permission_granted');
    return pluginGranted === true;
  } catch (err) {
    errLog('ensureNotificationPermission failed: %O', err);
    return false;
  }
}

/**
 * Invoke the Tauri shell to show a native OS notification. No-op when the
 * app is running outside Tauri (e.g. Vitest / pure-web dev server).
 */
export async function showNativeNotification(args: ShowNativeNotificationArgs): Promise<void> {
  if (!isTauri()) {
    log('not running in tauri, skipping %o', args);
    return;
  }
  try {
    await invoke('plugin:notification|notify', {
      options: {
        title: args.title,
        body: args.body,
        sound: 'default',
      },
    });
  } catch (err) {
    errLog('plugin:notification|notify failed: %O', err);
  }
}
