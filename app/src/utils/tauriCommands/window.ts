/**
 * Window management commands.
 */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { isTauri } from './common';

/**
 * Show the main window
 */
export async function showWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  await window.show();
  await window.unminimize();
  await window.setFocus();
}

/**
 * Hide the main window
 */
export async function hideWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().hide();
}

/**
 * Toggle window visibility
 */
export async function toggleWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  const visible = await window.isVisible();
  if (visible) {
    await window.hide();
    return;
  }
  await window.show();
  await window.unminimize();
  await window.setFocus();
}

/**
 * Check if window is visible
 */
export async function isWindowVisible(): Promise<boolean> {
  if (!isTauri()) {
    return true; // In browser, window is always visible
  }

  return await getCurrentWindow().isVisible();
}

/**
 * Minimize the window
 */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().minimize();
}

/**
 * Maximize or unmaximize the window
 */
export async function maximizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  await window.toggleMaximize();
}

/**
 * Close the window (minimizes to tray on macOS)
 */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().close();
}

/**
 * Set the window title
 */
export async function setWindowTitle(title: string): Promise<void> {
  if (!isTauri()) {
    document.title = title;
    return;
  }

  await getCurrentWindow().setTitle(title);
}

/**
 * Show the notch activity indicator (the transparent macOS NSPanel pill that
 * surfaces live voice/agent status). No-op outside Tauri; the underlying Rust
 * command is itself a no-op on non-macOS.
 */
export async function notchWindowShow(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  try {
    await invoke('notch_window_show');
  } catch (err) {
    console.warn('[notch] show failed', err);
  }
}

/**
 * Hide (drop) the notch activity indicator.
 */
export async function notchWindowHide(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  try {
    await invoke('notch_window_hide');
  } catch (err) {
    console.warn('[notch] hide failed', err);
  }
}

/**
 * Sync the notch indicator's visibility to the always-on listening state. The
 * notch is the always-on listening HUD, so it is only shown while always-on
 * listening is enabled. Called on app boot and whenever the Settings toggle
 * flips so the pill never lingers once listening is turned off.
 */
export async function syncNotchVisibility(alwaysOnEnabled: boolean): Promise<void> {
  if (alwaysOnEnabled) {
    await notchWindowShow();
  } else {
    await notchWindowHide();
  }
}
