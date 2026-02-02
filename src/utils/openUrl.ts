import { isTauri } from '@tauri-apps/api/core';
import { openUrl as tauriOpenUrl } from '@tauri-apps/plugin-opener';

/**
 * Opens a URL in the default browser or app.
 * Works in both Tauri desktop app and regular browser environments.
 */
export const openUrl = async (url: string): Promise<void> => {
  // Check if we're running in Tauri desktop app
  if (isTauri()) {
    try {
      await tauriOpenUrl(url);
      return;
    } catch (error) {
      console.error('Failed to open URL with Tauri:', error);
      // Fall through to browser fallback
    }
  }

  // Browser fallback
  window.open(url, '_blank', 'noopener,noreferrer');
};
