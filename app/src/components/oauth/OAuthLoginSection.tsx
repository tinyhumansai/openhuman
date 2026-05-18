import { useT } from '../../lib/i18n/I18nContext';
import OAuthProviderButton from './OAuthProviderButton';
import { oauthProviderConfigs } from './providerConfigs';

interface OAuthLoginSectionProps {
  className?: string;
  disabled?: boolean;
  showTelegram?: boolean;
}

const OAuthLoginSection = ({ className = '', disabled = false }: OAuthLoginSectionProps) => {
  const { t } = useT();
  return (
    <div className={`space-y-4 ${className}`}>
      <div className="space-y-3">
        <h3 className="block w-full text-center text-sm opacity-30 mb-3 font-semibold tracking-wide">
          {t('oauth.login.continueWith')}
        </h3>
        <div className="grid grid-cols-2 gap-3">
          {oauthProviderConfigs.map(provider => (
            <OAuthProviderButton
              key={provider.id}
              provider={provider}
              disabled={disabled}
              className="w-full min-w-0"
            />
          ))}
        </div>
      </div>
    </div>
  );
};

export default OAuthLoginSection;
