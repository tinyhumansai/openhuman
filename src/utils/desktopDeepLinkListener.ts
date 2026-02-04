import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { consumeLoginToken } from '../services/api/authApi';
import { store } from '../store';
import { setToken } from '../store/authSlice';
import { IS_DEV } from './config';

/**
 * Handle a list of deep link URLs delivered by the Tauri deep-link plugin.
 * Parses `alphahuman://auth?token=...` URLs and exchanges the token for a
 * desktop session via the backend.
 */
const handleDeepLinkUrls = async (urls: string[] | null | undefined) => {
  if (!urls || urls.length === 0) {
    return;
  }

  const url = urls[0];

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'alphahuman:') {
      return;
    }
    // Harden: ensure this deep link is intended for auth handoff
    if (parsed.hostname !== 'auth') {
      return;
    }

    const token = parsed.searchParams.get('token');
    if (!token) {
      console.warn('[DeepLink] URL did not contain a token query parameter');
      return;
    }

    console.log('[DeepLink] Received token', token);

    try {
      // Bring app window to foreground so macOS users actually see completion.
      // (In this app, the window can start hidden and live in the tray.)
      await invoke('show_window');
    } catch (err) {
      // Not fatal; we still continue the auth flow.
      console.warn('[DeepLink] Failed to show window:', err);
    }

    const jwtToken = await consumeLoginToken(token);
    store.dispatch(setToken(jwtToken));

    // Navigate to post-login flow. We use HashRouter, so update the hash route.
    window.location.hash = '/onboarding';
  } catch (error) {
    console.error('[DeepLink] Failed to handle deep link URL:', url, error);
  }
};

/**
 * Set up listeners for deep links so that when the desktop app is opened
 * via a URL like `alphahuman://auth?token=...`, we can react to it.
 * Only works in Tauri desktop app environment.
 */
export const setupDesktopDeepLinkListener = async () => {
  // Only set up deep link listener in Tauri environment
  if (!coreIsTauri()) {
    return;
  }

  try {
    const startUrls = await getCurrent();
    if (startUrls) {
      await handleDeepLinkUrls(startUrls);
    }

    await onOpenUrl(urls => {
      void handleDeepLinkUrls(urls);
    });

    if (IS_DEV && typeof window !== 'undefined') {
      // window.__simulateDeepLink('alphahuman//auth?token=1234567890')
      (
        window as Window & { __simulateDeepLink?: (url: string) => Promise<void> }
      ).__simulateDeepLink = (url: string) => handleDeepLinkUrls([url]);
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
