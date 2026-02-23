import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { skillManager } from '../lib/skills/manager';
import { consumeLoginToken, fetchIntegrationTokens } from '../services/api/authApi';
import { store } from '../store';
import {
  decryptIntegrationTokens,
  hexToBase64,
  type IntegrationTokensPayload,
} from './integrationTokensCrypto';
import { setToken } from '../store/authSlice';
import { setSkillState } from '../store/skillsSlice';

function getCurrentUserId(): string | null {
  const state = store.getState();
  const explicitId = state.user.user?._id;
  if (explicitId) return explicitId;

  const token = state.auth.token;
  if (!token) return null;

  try {
    const parts = token.split('.');
    if (parts.length !== 3) return null;
    const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payloadJson = atob(payloadBase64);
    const payload = JSON.parse(payloadJson);
    return payload.tgUserId || payload.userId || payload.sub || null;
  } catch {
    return null;
  }
}

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

      const state = store.getState();
      const userId = getCurrentUserId();
      if (!userId) {
        console.warn('[DeepLink] Cannot fetch integration tokens: no current user id');
        return;
      }

      const encryptionKeyHex = state.auth.encryptionKeyByUser[userId];
      if (!encryptionKeyHex || typeof encryptionKeyHex !== 'string') {
        console.warn(
          '[DeepLink] Cannot fetch integration tokens: no encryption key found for user',
          userId
        );
        return;
      }

      const trimmedHex = encryptionKeyHex.trim().replace(/^0x/i, '');
      if (!trimmedHex || trimmedHex.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(trimmedHex)) {
        console.error(
          '[DeepLink] Cannot fetch integration tokens: encryption key must be non-empty hex (even length, [0-9a-fA-F])',
          { userId, encryptionKeyHex }
        );
        return;
      }

      let keyForBackend: string;
      try {
        keyForBackend = hexToBase64(encryptionKeyHex);
      } catch (e) {
        console.error(
          '[DeepLink] Cannot fetch integration tokens: encryption key conversion failed',
          { userId, encryptionKeyHex, error: e }
        );
        return;
      }
      if (!keyForBackend) {
        console.error(
          '[DeepLink] Cannot fetch integration tokens: encryption key produced empty base64',
          { userId, encryptionKeyHex }
        );
        return;
      }

      const response = await fetchIntegrationTokens(integrationId, keyForBackend);
      if (!response.success || !response.data?.encrypted) {
        console.warn(
          '[DeepLink] Integration tokens response missing encrypted payload for integration',
          integrationId
        );
        return;
      }

      const existingState = state.skills.skillStates[skillId] ?? {};
      store.dispatch(
        setSkillState({
          skillId,
          state: {
            ...existingState,
            oauthTokens: {
              ...(existingState.oauthTokens as Record<string, { encrypted: string }> | undefined),
              [integrationId]: { encrypted: response.data.encrypted },
            },
          },
        })
      );

      // For Gmail, pass decrypted access token so the skill uses it instead of the proxy
      let extraCredential: { accessToken?: string } | undefined;
      if (skillId === 'gmail') {
        try {
          const decryptedJson = await decryptIntegrationTokens(
            response.data.encrypted,
            encryptionKeyHex
          );
          const payload = JSON.parse(decryptedJson) as IntegrationTokensPayload;
          if (payload.accessToken) {
            extraCredential = { accessToken: payload.accessToken };
          }
        } catch (e) {
          console.warn('[DeepLink] Could not decrypt Gmail token for skill:', e);
        }
      }

      await skillManager.notifyOAuthComplete(skillId, integrationId, undefined, extraCredential);
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
      const win = window as Window & {
        __simulateDeepLink?: (url: string) => Promise<void>;
      };
      win.__simulateDeepLink = (url: string) => handleDeepLinkUrls([url]);
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
