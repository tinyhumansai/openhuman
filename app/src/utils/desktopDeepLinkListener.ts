import * as Sentry from '@sentry/react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { getCoreStateSnapshot, patchCoreStateSnapshot } from '../lib/coreState/store';
import { consumeLoginToken } from '../services/api/authApi';
import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
  failDeepLinkAuthProcessing,
} from '../store/deepLinkAuthState';
import { BILLING_DASHBOARD_URL } from './links';
import { evaluateOAuthAppVersionGate } from './oauthAppVersionGate';
import { openUrl } from './openUrl';
import { storeSession } from './tauriCommands';
import { isTauri as coreIsTauri } from './tauriCommands/common';

const SESSION_TOKEN_UPDATED_EVENT = 'core-state:session-token-updated';

const sanitizeOAuthDiagnosticValue = (
  value: string | null,
  fallback: string,
  maxLength = 80
): string => {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) {
    return fallback;
  }

  const safe = normalized.replace(/[^a-z0-9._-]/g, '_').slice(0, maxLength);
  return safe || fallback;
};

const getOAuthErrorMessage = (provider: string, errorCode: string): string => {
  if (provider === 'twitter') {
    if (errorCode === 'access_denied' || errorCode === 'user_denied') {
      return 'Twitter/X sign-in was cancelled. Try again and approve access to continue.';
    }

    return 'Twitter/X sign-in failed before OpenHuman received authorization. Check the Twitter Developer Portal app settings: OAuth 2.0 must be enabled, callback URL must match the backend redirect URL exactly, and the client ID, client secret, and requested scopes must match the OpenHuman backend configuration.';
  }

  if (errorCode === 'access_denied' || errorCode === 'user_denied') {
    return 'Sign-in was cancelled. Try again and approve access to continue.';
  }

  return 'OAuth sign-in failed before OpenHuman received authorization. Check the provider app settings and try again.';
};

const emitOAuthError = (provider: string, errorCode: string, message: string) => {
  console.warn('[DeepLink][oauth:error] OAuth provider returned an error', {
    provider,
    errorCode,
    message,
  });

  failDeepLinkAuthProcessing(message);
  window.dispatchEvent(
    new CustomEvent('oauth:error', { detail: { provider, errorCode, message } })
  );
};

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
    const coreState = getCoreStateSnapshot();
    if (!coreState.isBootstrapping || coreState.snapshot.sessionToken) {
      console.log('[DeepLink][auth] app ready', {
        attempt,
        hasToken: Boolean(coreState.snapshot.sessionToken),
        authBootstrapComplete: !coreState.isBootstrapping,
      });
      return;
    }
    await new Promise(resolve => setTimeout(resolve, delayMs));
  }
  console.warn('[DeepLink][auth] readiness timeout; continuing');
};

const applySessionToken = async (sessionToken: string): Promise<void> => {
  await storeSession(sessionToken, {});
  patchCoreStateSnapshot({ snapshot: { sessionToken } });
  window.dispatchEvent(new CustomEvent(SESSION_TOKEN_UPDATED_EVENT, { detail: { sessionToken } }));
};

/**
 * Handle an `openhuman://auth?token=...` deep link for login.
 */
const handleAuthDeepLink = async (parsed: URL) => {
  const token = parsed.searchParams.get('token');
  const key = parsed.searchParams.get('key');
  if (!token) {
    console.warn('[DeepLink] URL did not contain a token query parameter');
    failDeepLinkAuthProcessing('Sign-in callback was missing a token. Please try again.');
    return;
  }

  beginDeepLinkAuthProcessing();

  try {
    await focusMainWindow();
    await waitForAuthReadiness();

    const sessionToken = key === 'auth' ? token : await consumeLoginToken(token);
    await applySessionToken(sessionToken);

    window.location.hash = '/home';
    completeDeepLinkAuthProcessing();
  } catch (error) {
    console.error('[DeepLink][auth] failed to complete login:', error);
    failDeepLinkAuthProcessing('Sign-in failed. Please try again.');
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

    // Broadcast to the app in case any listeners still care about legacy
    // payment completion events.
    window.dispatchEvent(new CustomEvent('payment:success', { detail: { sessionId } }));

    await openUrl(BILLING_DASHBOARD_URL);
    window.location.hash = '/home';
  } else if (path === 'cancel') {
    console.log('[DeepLink] Payment cancelled');
    window.dispatchEvent(new CustomEvent('payment:cancel', {}));
    await openUrl(BILLING_DASHBOARD_URL);
    window.location.hash = '/home';
  } else {
    console.warn('[DeepLink] Unknown payment path:', path);
  }
};

/**
 * Handle `openhuman://oauth/success?...`
 * and `openhuman://oauth/error?error=...&provider=...` deep links.
 */
const handleOAuthDeepLink = async (parsed: URL) => {
  // pathname is "/success" or "/error" (hostname is "oauth")
  const path = parsed.pathname.replace(/^\/+/, '');

  await focusMainWindow();

  if (path === 'success') {
    const integrationId = parsed.searchParams.get('integrationId');
    const toolkit =
      parsed.searchParams.get('toolkit') ||
      parsed.searchParams.get('provider') ||
      parsed.searchParams.get('skillId');

    if (!integrationId) {
      // Do not log full URL — query can contain secrets.
      console.error('[DeepLink] OAuth success missing integrationId');
      return;
    }

    let versionGate: Awaited<ReturnType<typeof evaluateOAuthAppVersionGate>>;
    try {
      versionGate = await evaluateOAuthAppVersionGate();
    } catch (gateErr) {
      // Avoid bubbling: outer handler logs the raw URL and would leak query secrets.
      console.warn('[DeepLink] OAuth version gate failed; continuing OAuth', gateErr);
      versionGate = { ok: true };
    }

    if (!versionGate.ok) {
      const msg =
        versionGate.current === 'unknown'
          ? `OpenHuman could not verify this build against the minimum required for OAuth (${versionGate.minimum}). Install the latest release, then try connecting again.`
          : `This OpenHuman build (${versionGate.current}) is older than the minimum required for OAuth (${versionGate.minimum}). Install the latest release, then try connecting again.`;
      console.warn(`[DeepLink][oauth:stale-app] ${msg}`);
      try {
        await openUrl(versionGate.downloadUrl);
      } catch (e) {
        console.warn('[DeepLink] Could not open latest release URL', e);
      }
      Sentry.captureMessage(
        `OAuth blocked: stale app version ${versionGate.current}<${versionGate.minimum}`,
        {
          level: 'warning',
          tags: {
            component: 'desktopDeepLinkListener',
            current: versionGate.current,
            minimum: versionGate.minimum,
          },
        }
      );
      window.dispatchEvent(
        new CustomEvent('oauth:stale-app', {
          detail: {
            current: versionGate.current,
            minimum: versionGate.minimum,
            downloadUrl: versionGate.downloadUrl,
            integrationId,
          },
        })
      );
      return;
    }
    console.log(
      `[DeepLink] OAuth success for integration=${integrationId}${toolkit ? ` toolkit=${toolkit}` : ''}`
    );
    window.dispatchEvent(new CustomEvent('oauth:success', { detail: { integrationId, toolkit } }));
    window.location.hash = '/skills';
  } else if (path === 'error') {
    const provider = sanitizeOAuthDiagnosticValue(
      parsed.searchParams.get('provider'),
      'unknown',
      32
    );
    const errorCode = sanitizeOAuthDiagnosticValue(
      parsed.searchParams.get('error') || parsed.searchParams.get('error_code'),
      'unknown_error'
    );
    const message = getOAuthErrorMessage(provider, errorCode);
    emitOAuthError(provider, errorCode, message);
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
    // Avoid logging full `url` — OAuth callbacks can include sensitive query params.
    console.error('[DeepLink] Failed to handle deep link:', error);
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
      // window.__simulateDeepLink('openhuman://oauth/success?integrationId=69cafd0b103bd070232d3223&provider=notion')
      // window.__simulateDeepLink('openhuman://oauth/success?integrationId=69cafd0b103bd070232d3223&skillId=discord')
      const win = window as Window & { __simulateDeepLink?: (url: string) => Promise<void> };
      win.__simulateDeepLink = (url: string) => handleDeepLinkUrls([url]);
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
