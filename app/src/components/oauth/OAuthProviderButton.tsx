import { useState } from 'react';

import { getBackendUrl } from '../../services/backendUrl';
import type { OAuthProviderConfig } from '../../types/oauth';
import { IS_DEV } from '../../utils/config';
import { openUrl } from '../../utils/openUrl';
import { isTauri } from '../../utils/tauriCommands';

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

    setIsLoading(true);

    try {
      const backendUrl = await getBackendUrl();
      // In dev mode, request JSON response for browser testing — but NOT when
      // running in Tauri, where the backend must redirect to openhuman:// deep link.
      const useJsonResponse = IS_DEV && !isTauri();
      const loginUrl = `${backendUrl}/auth/${provider.id}/login${useJsonResponse ? '?responseType=json' : ''}`;

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
