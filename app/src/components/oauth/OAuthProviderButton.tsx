import { useEffect, useState } from 'react';

import { getBackendUrl } from '../../services/backendUrl';
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

const OAuthProviderButton = ({
  provider,
  className = '',
  disabled: externalDisabled = false,
  onClickOverride,
}: OAuthProviderButtonProps) => {
  const [isLoading, setIsLoading] = useState(false);

  useEffect(() => {
    if (!isLoading) return;

    const reset = () => setIsLoading(false);

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
    };

    // Backup path: macOS Spaces / virtual desktops sometimes restore window
    // focus without firing a `focus` event. `visibilitychange` is the more
    // reliable signal there.
    const handleVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      if (skipDuringDeepLink('visibilitychange')) return;
      console.debug(`[oauth-button][${provider.id}] visibilitychange visible → reset isLoading`);
      reset();
    };

    const timer = window.setTimeout(() => {
      console.debug(`[oauth-button][${provider.id}] timeout → reset isLoading`);
      reset();
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

    setIsLoading(true);

    try {
      const backendUrl = await getBackendUrl();
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
    } catch (error) {
      console.error(`Failed to initiate ${provider.name} OAuth login:`, error);
      setIsLoading(false);
    }
  };

  const isDisabled = externalDisabled || isLoading;
  const IconComponent = provider.icon;

  return (
    <button
      onClick={handleOAuthLogin}
      disabled={isDisabled}
      className={`flex min-w-0 items-center justify-center space-x-3 ${provider.color} ${provider.hoverColor} text-sm font-medium py-2.5 px-4 rounded-xl transition-all duration-300 hover:shadow-medium hover:scale-[1.02] active:scale-[0.98] disabled:hover:scale-100 disabled:opacity-50 disabled:cursor-not-allowed ${className}`}>
      {isLoading ? (
        <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-current"></div>
      ) : (
        <IconComponent className="w-5 h-5" />
      )}
      <span className={provider.textColor}>{isLoading ? 'Connecting...' : provider.name}</span>
    </button>
  );
};

export default OAuthProviderButton;
