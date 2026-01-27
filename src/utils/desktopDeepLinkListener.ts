import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
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
      console.warn('Deep link URL did not contain a token query parameter');
      return;
    }

    const response = await fetch(`${BACKEND_URL}/api/auth/desktop-exchange`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token }),
    });

    if (!response.ok) {
      console.error('Token exchange failed:', response.status);
      return;
    }

    const data = (await response.json()) as {
      sessionToken?: string;
      user?: { id: string; username: string; firstName?: string };
    };

    if (!data.sessionToken) {
      console.error('Backend did not return a sessionToken');
      return;
    }

    // Persist session so the app can use it after navigation.
    localStorage.setItem('sessionToken', data.sessionToken);
    if (data.user) {
      localStorage.setItem('user', JSON.stringify(data.user));
    }

    // Navigate to post-login flow. This listener runs outside the React
    // router context, so we assign the path directly and reload.
    window.location.replace('/onboarding/step1');
  } catch (error) {
    console.error('Failed to handle deep link URL:', url, error);
  }
};

/**
 * Set up listeners for deep links so that when the desktop app is opened
 * via a URL like `outsourced://auth?token=...`, we can react to it.
 */
export const setupDesktopDeepLinkListener = async () => {
  // Guard against running in plain web browser without Tauri.
  if (!(window as any).__TAURI__) {
    return;
  }

  const startUrls = await getCurrent();
  if (startUrls) {
    await handleDeepLinkUrls(startUrls);
  }

  await onOpenUrl(event => {
    void handleDeepLinkUrls(event);
  });
};

