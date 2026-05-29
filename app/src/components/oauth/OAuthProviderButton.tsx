import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { checkBackendHealthy } from '../../services/backendHealth';
import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
  getDeepLinkAuthState,
} from '../../store/deepLinkAuthState';
import type { OAuthProviderConfig } from '../../types/oauth';
import { IS_DEV } from '../../utils/config';
import { handleDeepLinkUrls } from '../../utils/desktopDeepLinkListener';
import { startLoopbackOauthListener } from '../../utils/loopbackOauthListener';
import { prepareOAuthLoginLaunch } from '../../utils/oauthAppVersionGate';
import { openUrl } from '../../utils/openUrl';
import { isTauri } from '../../utils/tauriCommands';

interface OAuthProviderButtonProps {
  provider: OAuthProviderConfig;
  className?: string;
  disabled?: boolean;
  onClickOverride?: () => void;
}

// Reset the loading state if the OAuth round-trip never completes — covers
// the case where the user cancels in the system browser, or the backend
// redirect fails so the `openhuman://` deep link never fires.
const OAUTH_LOADING_TIMEOUT_MS = 90_000;

// Pre-flight budget for `/health` before we open the system browser. Kept
// short so a healthy backend adds barely any perceptible click→browser delay,
// while an outage (Cloudflare 504, DNS, offline) is caught fast enough that
// the user never sees the broken provider page.
const OAUTH_PREFLIGHT_TIMEOUT_MS = 4_000;

const BACKEND_UNAVAILABLE_MESSAGE =
  'OpenHuman cloud sign-in is temporarily unavailable. Please try again in a few minutes.';

const log = debug('oauth:button');
const warnLog = debug('oauth:button:warn');
const errorLog = debug('oauth:button:error');

const SAFE_READINESS_STARTUP_MESSAGE_PREFIXES = [
  'Finish choosing how OpenHuman runs',
  'OpenHuman could not reach its local runtime',
];

const getSafeReadinessStartupMessage = (error: unknown): string | null => {
  if (!(error instanceof Error)) {
    return null;
  }

  const message = error.message.trim();
  if (!message) {
    return null;
  }

  return SAFE_READINESS_STARTUP_MESSAGE_PREFIXES.some(prefix => message.startsWith(prefix))
    ? message
    : null;
};

const getOAuthStartupFailureMessage = (provider: OAuthProviderConfig, error?: unknown): string => {
  const readinessMessage = getSafeReadinessStartupMessage(error);
  if (readinessMessage) {
    return readinessMessage;
  }

  if (provider.id === 'twitter') {
    return 'Twitter/X sign-in could not start. Check that the Twitter OAuth app callback URL, client ID/secret, and requested scopes match the OpenHuman backend, then try again.';
  }

  return `${provider.name} sign-in could not start. Please try again.`;
};

const summarizeOAuthStartupError = (error: unknown): string => {
  if (!(error instanceof Error)) {
    return typeof error;
  }

  // Keep diagnostics useful without leaking URLs or query parameters from host
  // opener errors.
  const redactedMessage = error.message
    .replace(/https?:\/\/\S+/g, '[redacted-url]')
    .replace(/openhuman:\/\/\S+/g, '[redacted-deep-link]');

  return `${error.name}: ${redactedMessage.slice(0, 160)}`;
};

const OAuthProviderButton = ({
  provider,
  className = '',
  disabled: externalDisabled = false,
  onClickOverride,
}: OAuthProviderButtonProps) => {
  const { t } = useT();
  const [isLoading, setIsLoading] = useState(false);
  const [startupError, setStartupError] = useState<string | null>(null);
  // Tracks whether the user actually got dispatched to the system browser on
  // this attempt. Lets the focus/visibility handlers distinguish "user came
  // back from the browser" (probe for backend health) from "click never even
  // reached openUrl" (no probe needed — we already set a startup error).
  const browserOpenedRef = useRef(false);

  useEffect(() => {
    if (!isLoading) return;

    const reset = () => setIsLoading(false);

    // Confirm backend health when the user returns without a deep-link
    // callback. Healthy → silent reset (user just cancelled in the browser).
    // Unhealthy → surface a clear banner so the user understands why the
    // browser landed on an error page (issue #1985).
    const probeBackendOnReturn = (label: string) => {
      if (!browserOpenedRef.current) return;
      // Consume the flag so the second of a focus/visibilitychange pair (macOS
      // can fire both back-to-back when returning from the system browser)
      // becomes a no-op instead of triggering a redundant concurrent probe.
      browserOpenedRef.current = false;
      void checkBackendHealthy()
        .then(result => {
          if (!result.healthy) {
            warnLog('[%s] %s probe -> backend unhealthy %o', provider.id, label, {
              reason: result.reason,
              latencyMs: result.latencyMs,
              status: 'status' in result ? result.status : undefined,
            });
            setStartupError(BACKEND_UNAVAILABLE_MESSAGE);
          } else {
            log('[%s] %s probe -> backend healthy %o', provider.id, label, {
              status: result.status,
              latencyMs: result.latencyMs,
            });
          }
        })
        .catch(err => {
          // checkBackendHealthy already swallows network/abort errors and
          // turns them into a result; reaching this branch is unexpected.
          log('[%s] %s probe threw %o', provider.id, label, err);
        });
    };

    // Skip reset when a deep-link auth round-trip is already in flight — the
    // OAuth callback flips `isProcessing=true` AFTER the OS focus event fires,
    // and resetting first would briefly re-enable the button mid-redirect.
    const skipDuringDeepLink = (label: string) => {
      if (getDeepLinkAuthState().isProcessing) {
        log('[%s] %s - skip (deep-link processing)', provider.id, label);
        return true;
      }
      return false;
    };

    // Fast path: window focus fires when the user returns from the system
    // browser. On most platforms this lifts the loading state immediately.
    const handleFocus = () => {
      if (skipDuringDeepLink('focus')) return;
      log('[%s] window focus -> reset isLoading', provider.id);
      reset();
      probeBackendOnReturn('focus');
    };

    // Backup path: macOS Spaces / virtual desktops sometimes restore window
    // focus without firing a `focus` event. `visibilitychange` is the more
    // reliable signal there.
    const handleVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      if (skipDuringDeepLink('visibilitychange')) return;
      log('[%s] visibilitychange visible -> reset isLoading', provider.id);
      reset();
      probeBackendOnReturn('visibilitychange');
    };

    const timer = window.setTimeout(() => {
      log('[%s] timeout -> reset isLoading', provider.id);
      reset();
      // 90s with no deep-link is a strong "something went wrong" signal even
      // if the user never refocused the app. Probe so we can attribute it.
      probeBackendOnReturn('timeout');
    }, OAUTH_LOADING_TIMEOUT_MS);

    window.addEventListener('focus', handleFocus);
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      window.clearTimeout(timer);
      window.removeEventListener('focus', handleFocus);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [isLoading, provider.id]);

  const handleOAuthLogin = async () => {
    if (onClickOverride) {
      onClickOverride();
      return;
    }

    if (externalDisabled || isLoading) return;

    log('[%s] starting OAuth login (isTauri=%s)', provider.id, isTauri());

    setStartupError(null);
    setIsLoading(true);
    beginDeepLinkAuthProcessing();
    browserOpenedRef.current = false;

    try {
      // Fail-fast pre-flight: hitting `api.tinyhumans.ai/health` before opening
      // the browser lets us catch Cloudflare 504s / DNS outages immediately
      // (issue #1985) instead of sending the user into a system browser that
      // lands on a gateway-error page with no path back into the app.
      const preflight = await checkBackendHealthy({ timeoutMs: OAUTH_PREFLIGHT_TIMEOUT_MS });
      if (!preflight.healthy) {
        warnLog('[%s] preflight -> backend unhealthy %o', provider.id, {
          reason: preflight.reason,
          latencyMs: preflight.latencyMs,
          status: 'status' in preflight ? preflight.status : undefined,
        });
        completeDeepLinkAuthProcessing();
        setStartupError(BACKEND_UNAVAILABLE_MESSAGE);
        setIsLoading(false);
        return;
      }

      if (isTauri()) {
        await prepareOAuthLoginLaunch();
      }

      // Reuse the URL the preflight already resolved — `getBackendUrl()` may
      // hit a Tauri IPC round-trip and the result hasn't changed within a
      // single click handler.
      const backendUrl = preflight.backendUrl;
      // Prefer a loopback HTTP redirect (RFC 8252) over the openhuman:// deep
      // link: deep links are unpredictable on Linux/Windows and rely on
      // single-instance forwarding through a named pipe (#1130). If bind
      // fails (port in use, not in Tauri, etc.) we fall back to the legacy
      // deep-link path the backend already supports.
      const loopback = isTauri() ? await startLoopbackOauthListener() : null;
      const loginUrlBase = `${backendUrl}/auth/${provider.id}/login`;
      const params = new URLSearchParams();
      // `responseType=json` makes the backend return JSON in the browser tab
      // instead of redirecting — useful as a pre-loopback dev workaround, but
      // it shortcircuits the redirect so the loopback listener never fires.
      // Only set it when we have no loopback handle (web build, or bind failed).
      if (IS_DEV && !loopback) params.set('responseType', 'json');
      if (loopback) params.set('redirectUri', loopback.redirectUri);
      const loginUrl = params.toString() ? `${loginUrlBase}?${params}` : loginUrlBase;

      if (loopback) {
        // Race the loopback callback against the existing focus/timeout reset
        // path. Browser hits 127.0.0.1 -> shell emits event -> we feed the URL
        // through the same handler the openhuman:// path uses, so token
        // exchange and CoreStateProvider commit logic stays in one place.
        void loopback
          .awaitCallback()
          .then(callbackUrl => {
            const synthetic = callbackUrl.replace(
              /^https?:\/\/127\.0\.0\.1:\d+\/auth/,
              'openhuman://auth'
            );
            void handleDeepLinkUrls([synthetic]);
          })
          .catch(err => {
            warnLog('[%s] loopback callback failed', provider.id, err);
            const isTimeout = err instanceof Error && err.message.includes('timed out');
            if (isTimeout) {
              setIsLoading(false);
              setStartupError(t('oauth.button.loopbackTimeout'));
            }
          });
      }

      if (IS_DEV) {
        console.log(`[dev] OAuth debug mode enabled. OAuth URL: ${loginUrl}`);
        console.log('[dev] In debug mode, OAuth will return JSON response instead of redirect.');
        console.log(
          '[dev] After OAuth completion, copy the loginToken and use: window.__simulateDeepLink("openhuman://auth?token=YOUR_TOKEN")'
        );
      }

      // Desktop (Tauri): use system browser → backend OAuth → deep link back to app
      if (isTauri()) {
        await openUrl(loginUrl);
      } else {
        // Web fallback: direct OAuth flow in current window
        window.location.href = loginUrl;
      }
      browserOpenedRef.current = true;
      completeDeepLinkAuthProcessing();
    } catch (error) {
      completeDeepLinkAuthProcessing();
      const message = getOAuthStartupFailureMessage(provider, error);
      errorLog('[%s] OAuth startup failed %o', provider.id, {
        provider: provider.id,
        providerName: provider.name,
        reason: summarizeOAuthStartupError(error),
        guidance: message,
      });
      setStartupError(message);
      setIsLoading(false);
    }
  };

  const isDisabled = externalDisabled || isLoading;
  const IconComponent = provider.icon;

  return (
    <div className="min-w-0">
      <button
        onClick={handleOAuthLogin}
        disabled={isDisabled}
        className={`flex min-w-0 items-center justify-center space-x-3 ${provider.color} ${provider.hoverColor} text-sm font-medium py-2.5 px-4 rounded-xl transition-all duration-300 hover:shadow-medium hover:scale-[1.02] active:scale-[0.98] disabled:hover:scale-100 disabled:opacity-50 disabled:cursor-not-allowed ${className}`}>
        {isLoading ? (
          <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-current"></div>
        ) : (
          <IconComponent className="w-5 h-5" />
        )}
        <span className={provider.textColor}>
          {isLoading ? t('oauth.button.connecting') : provider.name}
        </span>
      </button>
      {startupError ? (
        <p role="alert" className="mt-2 text-xs leading-5 text-red-600">
          {startupError}
        </p>
      ) : null}
    </div>
  );
};

export default OAuthProviderButton;
