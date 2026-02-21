import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { skillManager } from '../lib/skills/manager';
import { consumeLoginToken } from '../services/api/authApi';
import { store } from '../store';
import { setToken } from '../store/authSlice';

/**
 * Handle an `alphahuman://auth?token=...` deep link for login.
 */
const handleAuthDeepLink = async (parsed: URL) => {
  const token = parsed.searchParams.get('token');
  if (!token) {
    console.warn('[DeepLink] URL did not contain a token query parameter');
    return;
  }

  console.log('[DeepLink] Received auth token');

  try {
    await invoke('show_window');
  } catch (err) {
    console.warn('[DeepLink] Failed to show window:', err);
  }

  const jwtToken = await consumeLoginToken(token);
  store.dispatch(setToken(jwtToken));
  window.location.hash = '/onboarding';
};

/**
 * Handle `alphahuman://payment/success?session_id=...` deep links.
 * Fired when a Stripe checkout session completes and the browser redirects
 * back to the desktop app.
 */
const handlePaymentDeepLink = async (parsed: URL) => {
  const path = parsed.pathname.replace(/^\/+/, '');

  try {
    await invoke('show_window');
  } catch {
    // Not fatal
  }

  if (path === 'success') {
    const sessionId = parsed.searchParams.get('session_id');

    if (!sessionId) {
      console.warn('[DeepLink] Payment success missing session_id');
      return;
    }

    console.log('[DeepLink] Payment success, session_id:', sessionId);

    // Broadcast to the app so billing components can react
    window.dispatchEvent(
      new CustomEvent('payment:success', { detail: { sessionId } }),
    );

    // Navigate to billing settings to show confirmation
    window.location.hash = '/settings/billing';
  } else if (path === 'cancel') {
    console.log('[DeepLink] Payment cancelled');
    window.dispatchEvent(new CustomEvent('payment:cancel', {}));
    window.location.hash = '/settings/billing';
  } else {
    console.warn('[DeepLink] Unknown payment path:', path);
  }
};

/**
 * Handle `alphahuman://oauth/success?integrationId=...&skillId=...`
 * and `alphahuman://oauth/error?error=...&provider=...` deep links.
 */
const handleOAuthDeepLink = async (parsed: URL) => {
  // pathname is "/success" or "/error" (hostname is "oauth")
  const path = parsed.pathname.replace(/^\/+/, '');

  try {
    await invoke('show_window');
  } catch {
    // Not fatal
  }

  if (path === 'success') {
    const integrationId = parsed.searchParams.get('integrationId');
    const skillId = parsed.searchParams.get('skillId');

    if (!integrationId || !skillId) {
      console.error('[DeepLink] OAuth success missing integrationId or skillId', parsed.href);
      return;
    }

    console.log(`[DeepLink] OAuth success for skill=${skillId} integration=${integrationId}`);

    try {
      await skillManager.notifyOAuthComplete(skillId, integrationId);
    } catch (err) {
      console.error('[DeepLink] Failed to notify OAuth complete:', err);
    }
  } else if (path === 'error') {
    const error = parsed.searchParams.get('error') ?? 'Unknown error';
    const provider = parsed.searchParams.get('provider') ?? 'unknown';
    console.error(`[DeepLink] OAuth error for provider=${provider}: ${error}`);
  } else {
    console.warn('[DeepLink] Unknown OAuth path:', path);
  }
};

/**
 * Handle a list of deep link URLs delivered by the Tauri deep-link plugin.
 * Routes to the appropriate handler based on the URL hostname:
 *   - `alphahuman://auth?token=...` → login flow
 *   - `alphahuman://oauth/success?...` → OAuth completion
 *   - `alphahuman://oauth/error?...` → OAuth failure
 *   - `alphahuman://payment/success?session_id=...` → Stripe payment confirmation
 *   - `alphahuman://payment/cancel` → Stripe payment cancellation
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

    switch (parsed.hostname) {
      case 'auth':
        await handleAuthDeepLink(parsed);
        break;
      case 'oauth':
        await handleOAuthDeepLink(parsed);
        break;
      case 'payment':
        await handlePaymentDeepLink(parsed);
        break;
      default:
        console.warn('[DeepLink] Unknown deep link hostname:', parsed.hostname);
        break;
    }
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

    if (typeof window !== 'undefined') {
      // window.__simulateDeepLink('alphahuman://auth?token=1234567890')
      // window.__simulateDeepLink('alphahuman://oauth/success?integrationId=6989ef9c8e8bf1b6d991a08c&skillId=notion')
      (
        window as Window & { __simulateDeepLink?: (url: string) => Promise<void> }
      ).__simulateDeepLink = (url: string) => handleDeepLinkUrls([url]);
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
