import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { invoke } from '@tauri-apps/api/core';
import { BACKEND_URL } from './config';

/**
 * Handle a list of deep link URLs delivered by the Tauri deep-link plugin.
 * Parses `outsourced://auth?token=...` URLs and exchanges the token for a
 * desktop session via the backend.
 */
const handleDeepLinkUrls = async (urls: string[] | null | undefined) => {
  if (!urls || urls.length === 0) {
    return;
  }

  const url = urls[0];

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'outsourced:') {
      return;
    }

    const token = parsed.searchParams.get('token');
    if (!token) {
      console.warn('[DeepLink] URL did not contain a token query parameter');
      return;
    }

    console.log('[DeepLink] Received token');

    let sessionToken: string | undefined;
    let user: { id: string; username: string; firstName?: string } | undefined;

    try {
      // Use Tauri invoke to call Rust backend (bypasses CORS)
      const data = await invoke<{
        sessionToken?: string;
        user?: { id: string; username: string; firstName?: string };
      }>('exchange_token', { backendUrl: BACKEND_URL, token });

      sessionToken = data.sessionToken;
      user = data.user;
    } catch (err) {
      console.warn('[DeepLink] Token exchange failed:', err);
    }

    // If the backend didn't return a session, store the raw token so the
    // login flow can proceed. This path is used during development when
    // the backend server is not yet running.
    if (!sessionToken) {
      sessionToken = token;
    }

    localStorage.setItem('sessionToken', sessionToken);
    localStorage.setItem('deepLinkHandled', 'true');
    if (user) {
      localStorage.setItem('user', JSON.stringify(user));
    }

    // Navigate to post-login flow. This listener runs outside the React
    // router context, so we assign the path directly and reload.
    window.location.replace('/onboarding/step1');
  } catch (error) {
    console.error('[DeepLink] Failed to handle deep link URL:', url, error);
  }
};

/**
 * Set up listeners for deep links so that when the desktop app is opened
 * via a URL like `outsourced://auth?token=...`, we can react to it.
 */
export const setupDesktopDeepLinkListener = async () => {
  try {
    const startUrls = await getCurrent();
    if (startUrls && !localStorage.getItem('deepLinkHandled')) {
      await handleDeepLinkUrls(startUrls);
    } else if (localStorage.getItem('deepLinkHandled')) {
      localStorage.removeItem('deepLinkHandled');
    }

    await onOpenUrl(urls => {
      void handleDeepLinkUrls(urls);
    });
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
