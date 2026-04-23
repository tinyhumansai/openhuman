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
 * Invoke the Tauri shell to show a native OS notification. No-op when the
 * app is running outside Tauri (e.g. Vitest / pure-web dev server).
 */
export async function showNativeNotification(args: ShowNativeNotificationArgs): Promise<void> {
  if (!isTauri()) {
    log('not running in tauri, skipping %o', args);
    return;
  }
  try {
    await invoke('show_native_notification', {
      title: args.title,
      body: args.body,
      tag: args.tag ?? null,
    });
  } catch (err) {
    errLog('show_native_notification failed: %O', err);
  }
}
