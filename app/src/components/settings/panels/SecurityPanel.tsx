import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { decideKeyringConsent, retryKeyringProbe } from '../../../services/keyringApi';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsRow, SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const MODE_BADGE_VARIANT: Record<string, 'success' | 'warning' | 'neutral' | 'danger'> = {
  os_keyring: 'success',
  local_encrypted: 'warning',
  consent_pending: 'neutral',
  declined: 'danger',
};

const SecurityPanel = () => {
  const { snapshot } = useCoreState();
  const { t } = useT();
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const keyringStatus = snapshot.keyringStatus;
  const modeBadgeVariant =
    MODE_BADGE_VARIANT[keyringStatus.activeMode] ?? MODE_BADGE_VARIANT.consent_pending;

  const handleRetryProbe = async () => {
    setIsLoading(true);
    setError(null);
    try {
      await retryKeyringProbe();
    } catch {
      setError(t('keyring.settings.retryFailed'));
    } finally {
      setIsLoading(false);
    }
  };

  const handleConsentChange = async (mode: 'local_encrypted' | 'declined') => {
    setIsLoading(true);
    setError(null);
    try {
      await decideKeyringConsent(mode);
    } catch {
      setError(t('keyring.consent.error'));
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <SettingsPanel description={t('pages.settings.account.securityDesc')}>
      <>
        {/* Storage mode */}
        <SettingsSection title={t('keyring.settings.storageMode')}>
          <SettingsRow
            label={t('keyring.settings.storageMode')}
            control={
              <div className="flex items-center gap-3">
                <SettingsBadge variant={modeBadgeVariant}>
                  {t(
                    `keyring.settings.mode.${keyringStatus.activeMode}` as Parameters<typeof t>[0]
                  )}
                </SettingsBadge>
                <span className="text-xs text-content-muted">
                  {t('keyring.settings.backend')}: {keyringStatus.backendName}
                </span>
              </div>
            }
          />
        </SettingsSection>

        {/* Availability */}
        <SettingsSection title={t('keyring.settings.availability')}>
          <div className="px-4 py-3 space-y-3">
            <div className="flex items-center gap-2">
              <div
                className={`h-2 w-2 rounded-full ${keyringStatus.available ? 'bg-sage-500' : 'bg-amber-500'}`}
              />
              <span className="text-sm text-content-secondary">
                {keyringStatus.available
                  ? t('keyring.settings.available')
                  : t('keyring.settings.unavailable')}
              </span>
            </div>
            {keyringStatus.failureReason && (
              <p className="text-xs text-content-muted ml-4">{keyringStatus.failureReason}</p>
            )}
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void handleRetryProbe()}
              disabled={isLoading}>
              {isLoading ? t('keyring.consent.retrying') : t('keyring.settings.retryButton')}
            </Button>
          </div>
        </SettingsSection>

        {/* Consent management (only when keyring is unavailable) */}
        {!keyringStatus.available && (
          <SettingsSection title={t('keyring.settings.consentTitle')}>
            <div className="px-4 py-3 space-y-3">
              <p className="text-xs text-content-muted">
                {t('keyring.settings.consentDescription')}
              </p>
              <div className="flex flex-wrap gap-2">
                {keyringStatus.activeMode !== 'local_encrypted' && (
                  <Button
                    type="button"
                    variant="primary"
                    size="sm"
                    onClick={() => void handleConsentChange('local_encrypted')}
                    disabled={isLoading}>
                    {t('keyring.settings.grantConsent')}
                  </Button>
                )}
                {keyringStatus.activeMode !== 'declined' && (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => void handleConsentChange('declined')}
                    disabled={isLoading}>
                    {t('keyring.settings.revokeConsent')}
                  </Button>
                )}
              </div>
            </div>
          </SettingsSection>
        )}

        <SettingsStatusLine
          saving={isLoading}
          error={error}
          savingLabel={t('keyring.consent.retrying')}
        />
      </>
    </SettingsPanel>
  );
};

export default SecurityPanel;
