/**
 * Tauri Commands
 *
 * Helper functions for invoking Tauri commands from the frontend.
 */

import { invoke, isTauri as coreIsTauri } from '@tauri-apps/api/core';

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  // Tauri v2: prefer the official runtime check over window globals.
  return coreIsTauri();
};

/**
 * Start Telegram login via the widget in the system browser
 * The backend will redirect back to the app via deep link
 */
export async function startTelegramLogin(): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  await invoke('start_telegram_login');
}

/**
 * Start Telegram login with a custom backend URL
 */
export async function startTelegramLoginWithUrl(backendUrl: string): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  await invoke('start_telegram_login_with_url', { backendUrl });
}

/**
 * Exchange a login token for a session token
 */
export async function exchangeToken(
  backendUrl: string,
  token: string
): Promise<{ sessionToken: string; user: object }> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  return await invoke('exchange_token', { backendUrl, token });
}

/**
 * Get the current authentication state from Rust
 */
export async function getAuthState(): Promise<{
  is_authenticated: boolean;
  user: object | null;
}> {
  if (!isTauri()) {
    return { is_authenticated: false, user: null };
  }

  return await invoke('get_auth_state');
}

/**
 * Get the session token from secure storage
 */
export async function getSessionToken(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  return await invoke('get_session_token');
}

/**
 * Logout and clear session
 */
export async function logout(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('logout');
}

/**
 * Store session in secure storage
 */
export async function storeSession(
  token: string,
  user: object
): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('store_session', { token, user });
}

/**
 * Show the main window
 */
export async function showWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('show_window');
}

/**
 * Hide the main window
 */
export async function hideWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('hide_window');
}

/**
 * Toggle window visibility
 */
export async function toggleWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('toggle_window');
}

/**
 * Check if window is visible
 */
export async function isWindowVisible(): Promise<boolean> {
  if (!isTauri()) {
    return true; // In browser, window is always visible
  }

  return await invoke('is_window_visible');
}

/**
 * Minimize the window
 */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('minimize_window');
}

/**
 * Maximize or unmaximize the window
 */
export async function maximizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('maximize_window');
}

/**
 * Close the window (minimizes to tray on macOS)
 */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('close_window');
}

/**
 * Set the window title
 */
export async function setWindowTitle(title: string): Promise<void> {
  if (!isTauri()) {
    document.title = title;
    return;
  }

  await invoke('set_window_title', { title });
}
