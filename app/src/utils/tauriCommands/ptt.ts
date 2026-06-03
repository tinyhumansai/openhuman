/**
 * Push-to-talk (PTT) Tauri command wrappers.
 */
import { invoke } from '@tauri-apps/api/core';

import { isTauri } from './common';

/**
 * Register (or re-register) the global push-to-talk hotkey.
 */
export async function registerPttHotkey(shortcut: string): Promise<void> {
  if (!isTauri()) {
    console.debug('[ptt] registerPttHotkey: skipped — not running in Tauri');
    return;
  }
  console.debug('[ptt] registerPttHotkey: shortcut=%s', shortcut);
  await invoke<void>('register_ptt_hotkey', { shortcut });
  console.debug('[ptt] registerPttHotkey: done');
}

/**
 * Unregister the global push-to-talk hotkey.
 */
export async function unregisterPttHotkey(): Promise<void> {
  if (!isTauri()) {
    console.debug('[ptt] unregisterPttHotkey: skipped — not running in Tauri');
    return;
  }
  console.debug('[ptt] unregisterPttHotkey: invoking');
  await invoke<void>('unregister_ptt_hotkey');
  console.debug('[ptt] unregisterPttHotkey: done');
}

/**
 * Show or hide the PTT overlay window.
 */
export async function showPttOverlay(active: boolean, sessionId: number): Promise<void> {
  if (!isTauri()) {
    console.debug('[ptt] showPttOverlay: skipped — not running in Tauri');
    return;
  }
  console.debug('[ptt] showPttOverlay: active=%s sessionId=%d', active, sessionId);
  await invoke<void>('show_ptt_overlay', { active, sessionId });
}
