import { isTauri as coreIsTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { skillManager } from '../lib/skills/manager';
import { emitSkillStateChange } from '../lib/skills/skillEvents';
import { setSetupComplete as rpcSetSetupComplete, startSkill } from '../lib/skills/skillsApi';
import { consumeLoginToken } from '../services/api/authApi';
import { store } from '../store';
import { setToken } from '../store/authSlice';

const focusMainWindow = async () => {
  try {
    const window = getCurrentWindow();
    await window.show();
    await window.unminimize();
    await window.setFocus();
  } catch (err) {
    console.warn('[DeepLink] Failed to focus window:', err);
  }
};

const waitForAuthReadiness = async (maxAttempts = 10, delayMs = 150) => {
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const authState = store.getState().auth;
    if (authState.isAuthBootstrapComplete || authState.token) {
      console.log('[DeepLink][auth] app ready', {
        attempt,
        hasToken: Boolean(authState.token),
        authBootstrapComplete: authState.isAuthBootstrapComplete,
      });
      return;
    }
    await new Promise(resolve => setTimeout(resolve, delayMs));
  }
  console.warn('[DeepLink][auth] readiness timeout; continuing');
};

/**
 * Handle an `openhuman://auth?token=...` deep link for login.
 */
const handleAuthDeepLink = async (parsed: URL) => {
  const token = parsed.searchParams.get('token');
  const key = parsed.searchParams.get('key');
  if (!token) {
    console.warn('[DeepLink] URL did not contain a token query parameter');
    return;
  }

  console.log('[DeepLink][auth] received', {
    tokenLength: token.length,
    keyMode: parsed.searchParams.get('key') ?? 'consume',
  });

  await focusMainWindow();
  await waitForAuthReadiness();

  if (key === 'auth') {
    store.dispatch(setToken(token));
    console.log('[DeepLink][auth] bypass token applied');
    window.location.hash = '/home';
  } else {
    const jwtToken = await consumeLoginToken(token);
    store.dispatch(setToken(jwtToken));
    console.log('[DeepLink][auth] login token consumed');
    window.location.hash = '/home';
  }
};

/**
 * Handle `openhuman://payment/success?session_id=...` deep links.
 * Fired when a Stripe checkout session completes and the browser redirects
 * back to the desktop app.
 */
const handlePaymentDeepLink = async (parsed: URL) => {
  const path = parsed.pathname.replace(/^\/+/, '');

  await focusMainWindow();

  if (path === 'success') {
    const sessionId = parsed.searchParams.get('session_id');

    if (!sessionId) {
      console.warn('[DeepLink] Payment success missing session_id');
      return;
    }

    console.log('[DeepLink] Payment success, session_id:', sessionId);

    // Broadcast to the app so billing components can react
    window.dispatchEvent(new CustomEvent('payment:success', { detail: { sessionId } }));

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
 * Handle `openhuman://oauth/success?integrationId=...&skillId=...`
 * and `openhuman://oauth/error?error=...&provider=...` deep links.
 */
const handleOAuthDeepLink = async (parsed: URL) => {
  // pathname is "/success" or "/error" (hostname is "oauth")
  const path = parsed.pathname.replace(/^\/+/, '');

  await focusMainWindow();

  if (path === 'success') {
    const integrationId = parsed.searchParams.get('integrationId');
    const skillId = parsed.searchParams.get('skillId');

    if (!integrationId || !skillId) {
      console.error('[DeepLink] OAuth success missing integrationId or skillId', parsed.href);
      return;
    }

    console.log(`[DeepLink] OAuth success for skill=${skillId} integration=${integrationId}`);

    // 1. Persist setup completion
    await rpcSetSetupComplete(skillId, true).catch(err =>
      console.warn('[DeepLink] Failed to persist setup_complete via RPC:', err)
    );
    emitSkillStateChange(skillId);

    // 2. Start the skill in the core QuickJS runtime (if not already running)
    try {
      await startSkill(skillId);
      console.log(`[DeepLink] Skill '${skillId}' started in core runtime`);
    } catch (startErr) {
      console.warn(`[DeepLink] Could not start skill '${skillId}' in runtime:`, startErr);
    }

    // 3. Send oauth/complete to the running skill with the credential
    try {
      await skillManager.notifyOAuthComplete(skillId, integrationId);
      console.log(`[DeepLink] OAuth complete sent to skill '${skillId}'`);
    } catch (runtimeErr) {
      console.warn('[DeepLink] Runtime notify failed:', runtimeErr);
    }

    // 4. Trigger initial data sync
    try {
      await skillManager.triggerSync(skillId);
    } catch {
      // Non-critical
    }

    emitSkillStateChange(skillId);
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
 *   - `openhuman://auth?token=...` → login flow
 *   - `openhuman://oauth/success?...` → OAuth completion
 *   - `openhuman://oauth/error?...` → OAuth failure
 *   - `openhuman://payment/success?session_id=...` → Stripe payment confirmation
 *   - `openhuman://payment/cancel` → Stripe payment cancellation
 */
const handleDeepLinkUrls = async (urls: string[] | null | undefined) => {
  if (!urls || urls.length === 0) {
    return;
  }

  const url = urls[0];

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'openhuman:') {
      console.warn('[DeepLink] Ignoring unsupported protocol:', parsed.protocol);
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
 * via a URL like `openhuman://auth?token=...`, we can react to it.
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
      // window.__simulateDeepLink('openhuman://auth?token=1234567890')
      // window.__simulateDeepLink('openhuman://oauth/success?integrationId=69c34e6a103bd070232d2710&skillId=notion')
      const win = window as Window & { __simulateDeepLink?: (url: string) => Promise<void> };
      win.__simulateDeepLink = (url: string) => handleDeepLinkUrls([url]);
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
