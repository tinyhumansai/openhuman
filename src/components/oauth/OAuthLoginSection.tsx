import OAuthProviderButton from './OAuthProviderButton';
import TelegramLoginButton from '../TelegramLoginButton';
import { oauthProviderConfigs } from './providerConfigs';

interface OAuthLoginSectionProps {
  className?: string;
  disabled?: boolean;
  showTelegram?: boolean;
}

const OAuthLoginSection = ({
  className = '',
  disabled = false,
  showTelegram = true
}: OAuthLoginSectionProps) => {
  return (
    <div className={`space-y-4 ${className}`}>
      {/* OAuth Providers */}
      <div className="space-y-3">
        {oauthProviderConfigs.map((provider) => (
          <OAuthProviderButton
            key={provider.id}
            provider={provider}
            disabled={disabled}
          />
        ))}
      </div>

      {/* Divider */}
      {showTelegram && (
        <>
          <div className="relative flex items-center">
            <div className="flex-1 border-t border-white/20"></div>
            <div className="px-4 text-sm text-white/50 bg-transparent">or</div>
            <div className="flex-1 border-t border-white/20"></div>
          </div>

          {/* Telegram Login */}
          <TelegramLoginButton disabled={disabled} />
        </>
      )}
    </div>
  );
};

export default OAuthLoginSection;