import { useState } from 'react';
import type { OAuthProviderConfig } from '../../types/oauth';
import { openUrl } from '../../utils/openUrl';
import { isTauri } from '../../utils/tauriCommands';
import { IS_DEV } from '../../utils/config';

interface OAuthProviderButtonProps {
  provider: OAuthProviderConfig;
  className?: string;
  disabled?: boolean;
}

const OAuthProviderButton = ({
  provider,
  className = '',
  disabled: externalDisabled = false,
}: OAuthProviderButtonProps) => {
  const [isLoading, setIsLoading] = useState(false);

  const handleOAuthLogin = async () => {
    if (externalDisabled || isLoading) return;

    console.log(`Starting ${provider.name} OAuth login`, isTauri());

    if (IS_DEV) {
      console.log(`[dev] OAuth debug mode enabled. OAuth URL: ${provider.loginUrl}`);
      console.log('[dev] In debug mode, OAuth will return JSON response instead of redirect.');
      console.log('[dev] After OAuth completion, copy the loginToken and use: window.__simulateDeepLink("alphahuman://auth?token=YOUR_TOKEN")');
    }

    setIsLoading(true);

    try {
      // Desktop (Tauri): use system browser → backend OAuth → deep link back to app
      if (isTauri()) {
        await openUrl(provider.loginUrl);
      } else {
        // Web fallback: direct OAuth flow in current window
        window.location.href = provider.loginUrl;
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
      className={`w-full flex items-center justify-center space-x-3 ${provider.color} ${provider.hoverColor} ${provider.textColor} font-semibold py-4 rounded-xl transition-all duration-300 hover:shadow-medium hover:scale-[1.02] active:scale-[0.98] disabled:hover:scale-100 disabled:opacity-50 disabled:cursor-not-allowed ${className}`}
    >
      {isLoading ? (
        <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-current"></div>
      ) : (
        <IconComponent className="w-5 h-5" />
      )}
      <span>
        {isLoading ? 'Connecting...' : `Continue with ${provider.name}`}
      </span>
    </button>
  );
};

export default OAuthProviderButton;