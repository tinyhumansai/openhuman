import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { isTauri } from '../../utils/tauriCommands/common';

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

// The bundled tauri-plugin-notification's `permission_state` is hardcoded
// to `Granted` on desktop, so calls to `plugin:notification|*` cannot be
// trusted to reflect the real OS authorization state. We route through
// the dedicated `notification_permission_state` /
// `notification_permission_request` / `show_native_notification` Rust
// commands (see app/src-tauri/src/native_notifications/), which talk to
// `UNUserNotificationCenter` directly on macOS and surface real
// delivery errors instead of swallowing them.

// Maps the Rust commands' raw status string ("granted", "denied",
// "not_determined", "provisional", "ephemeral", "unknown") onto the
// frontend's three-state union. Provisional / ephemeral are treated as
// granted because the OS allows quiet delivery in those modes.
function mapBackendState(raw: string): NotificationPermissionState {
  const state = raw.toLowerCase();
  if (state === 'granted' || state === 'provisional' || state === 'ephemeral') return 'granted';
  if (state === 'denied') return 'denied';
  if (state === 'not_determined' || state === 'prompt' || state === 'default') return 'prompt';
  return 'unknown';
}

export async function getNotificationPermissionState(options?: {
  requestIfNeeded?: boolean;
}): Promise<NotificationPermissionState> {
  const requestIfNeeded = options?.requestIfNeeded ?? true;
  if (!isTauri()) {
    return 'not_tauri';
  }

  try {
    const stateRaw = await invoke<string>('notification_permission_state');
    const state = mapBackendState(String(stateRaw ?? 'unknown'));
    log('notification_permission_state raw=%s mapped=%s', stateRaw, state);

    if (state === 'granted' || state === 'denied') return state;
    if (!requestIfNeeded) return state;

    const requestRaw = await invoke<string>('notification_permission_request');
    const requested = mapBackendState(String(requestRaw ?? 'unknown'));
    log('notification_permission_request raw=%s mapped=%s', requestRaw, requested);
    return requested;
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
 *
 * On macOS the Rust command waits for
 * `UNUserNotificationCenter.add(...)`'s completion handler, so a resolved
 * `{ delivered: true }` means the OS accepted the request — not just
 * that an async dispatch was scheduled.
 */
export async function showNativeNotification(
  args: ShowNativeNotificationArgs
): Promise<ShowNativeNotificationResult> {
  if (!isTauri()) {
    log('not running in tauri, skipping %o', args);
    return { delivered: false, reason: 'not_tauri' };
  }
  try {
    await invoke('show_native_notification', {
      title: args.title,
      body: args.body,
      tag: args.tag ?? null,
    });
    log('show_native_notification success tag=%s', args.tag ?? 'none');
    return { delivered: true };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    errLog('show_native_notification failed: %O', err);
    return { delivered: false, reason: 'send_failed', error: message };
  }
}
