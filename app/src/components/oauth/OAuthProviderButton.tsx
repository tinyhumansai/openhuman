import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { checkBackendHealthy } from '../../services/backendHealth';
import { getDeepLinkAuthState } from '../../store/deepLinkAuthState';
import type { OAuthProviderConfig } from '../../types/oauth';
import { IS_DEV } from '../../utils/config';
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

const getOAuthStartupFailureMessage = (provider: OAuthProviderConfig): string => {
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
            console.warn(`[oauth-button][${provider.id}] ${label} probe → backend unhealthy`, {
              reason: result.reason,
              latencyMs: result.latencyMs,
              status: 'status' in result ? result.status : undefined,
            });
            setStartupError(BACKEND_UNAVAILABLE_MESSAGE);
          } else {
            console.debug(`[oauth-button][${provider.id}] ${label} probe → backend healthy`, {
              status: result.status,
              latencyMs: result.latencyMs,
            });
          }
        })
        .catch(err => {
          // checkBackendHealthy already swallows network/abort errors and
          // turns them into a result; reaching this branch is unexpected.
          console.debug(`[oauth-button][${provider.id}] ${label} probe threw`, err);
        });
    };

    // Skip reset when a deep-link auth round-trip is already in flight — the
    // OAuth callback flips `isProcessing=true` AFTER the OS focus event fires,
    // and resetting first would briefly re-enable the button mid-redirect.
    const skipDuringDeepLink = (label: string) => {
      if (getDeepLinkAuthState().isProcessing) {
        console.debug(`[oauth-button][${provider.id}] ${label} — skip (deep-link processing)`);
        return true;
      }
      return false;
    };

    // Fast path: window focus fires when the user returns from the system
    // browser. On most platforms this lifts the loading state immediately.
    const handleFocus = () => {
      if (skipDuringDeepLink('focus')) return;
      console.debug(`[oauth-button][${provider.id}] window focus → reset isLoading`);
      reset();
      probeBackendOnReturn('focus');
    };

    // Backup path: macOS Spaces / virtual desktops sometimes restore window
    // focus without firing a `focus` event. `visibilitychange` is the more
    // reliable signal there.
    const handleVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      if (skipDuringDeepLink('visibilitychange')) return;
      console.debug(`[oauth-button][${provider.id}] visibilitychange visible → reset isLoading`);
      reset();
      probeBackendOnReturn('visibilitychange');
    };

    const timer = window.setTimeout(() => {
      console.debug(`[oauth-button][${provider.id}] timeout → reset isLoading`);
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

    console.debug(`[oauth-button][${provider.id}] starting OAuth login (isTauri=${isTauri()})`);

    setStartupError(null);
    setIsLoading(true);
    browserOpenedRef.current = false;

    // Fail-fast pre-flight: hitting `api.tinyhumans.ai/health` before opening
    // the browser lets us catch Cloudflare 504s / DNS outages immediately
    // (issue #1985) instead of sending the user into a system browser that
    // lands on a gateway-error page with no path back into the app.
    const preflight = await checkBackendHealthy({ timeoutMs: OAUTH_PREFLIGHT_TIMEOUT_MS });
    if (!preflight.healthy) {
      console.warn(`[oauth-button][${provider.id}] preflight → backend unhealthy`, {
        reason: preflight.reason,
        latencyMs: preflight.latencyMs,
        status: 'status' in preflight ? preflight.status : undefined,
      });
      setStartupError(BACKEND_UNAVAILABLE_MESSAGE);
      setIsLoading(false);
      return;
    }

    try {
      // Reuse the URL the preflight already resolved — `getBackendUrl()` may
      // hit a Tauri IPC round-trip and the result hasn't changed within a
      // single click handler.
      const backendUrl = preflight.backendUrl;
      const loginUrl = `${backendUrl}/auth/${provider.id}/login${IS_DEV ? '?responseType=json' : ''}`;

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
    } catch (error) {
      const message = getOAuthStartupFailureMessage(provider);
      console.error(`[oauth-button][${provider.id}] OAuth startup failed`, {
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
