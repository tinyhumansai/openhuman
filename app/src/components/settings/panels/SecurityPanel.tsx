import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { decideKeyringConsent, retryKeyringProbe } from '../../../services/keyringApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const MODE_BADGE: Record<string, { label: string; className: string }> = {
  os_keyring: {
    label: 'keyring.settings.mode.osKeychain',
    className:
      'bg-sage-50 dark:bg-sage-500/10 text-sage-700 dark:text-sage-300 border-sage-200 dark:border-sage-500/30',
  },
  local_encrypted: {
    label: 'keyring.settings.mode.encryptedFile',
    className:
      'bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-200 dark:border-amber-500/30',
  },
  consent_pending: {
    label: 'keyring.settings.mode.consentPending',
    className:
      'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200 border-stone-200 dark:border-neutral-800',
  },
  declined: {
    label: 'keyring.settings.mode.declined',
    className:
      'bg-coral-50 dark:bg-coral-500/10 text-coral-700 dark:text-coral-300 border-coral-200 dark:border-coral-500/30',
  },
};

const SecurityPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot } = useCoreState();
  const { t } = useT();
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const keyringStatus = snapshot.keyringStatus;
  const modeBadge = MODE_BADGE[keyringStatus.activeMode] ?? MODE_BADGE.consent_pending;

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
    <div>
      <SettingsHeader
        title={t('keyring.settings.title')}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="space-y-6 p-4">
        {/* Storage mode */}
        <section>
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-200 mb-3">
            {t('keyring.settings.storageMode')}
          </h3>
          <div className="flex items-center gap-3">
            <span
              className={`inline-flex items-center rounded-full border px-3 py-1 text-xs font-medium ${modeBadge.className}`}>
              {t(modeBadge.label)}
            </span>
            <span className="text-xs text-stone-500 dark:text-stone-400">
              {t('keyring.settings.backend')}: {keyringStatus.backendName}
            </span>
          </div>
        </section>

        {/* Availability */}
        <section>
          <h3 className="text-sm font-medium text-stone-700 dark:text-stone-200 mb-3">
            {t('keyring.settings.availability')}
          </h3>
          <div className="rounded-lg bg-stone-100 dark:bg-stone-800/60 p-4">
            <div className="flex items-center gap-2 mb-2">
              <div
                className={`h-2 w-2 rounded-full ${keyringStatus.available ? 'bg-sage-500' : 'bg-amber-500'}`}
              />
              <span className="text-sm text-stone-700 dark:text-stone-200">
                {keyringStatus.available
                  ? t('keyring.settings.available')
                  : t('keyring.settings.unavailable')}
              </span>
            </div>
            {keyringStatus.failureReason && (
              <p className="text-xs text-stone-500 dark:text-stone-400 ml-4">
                {keyringStatus.failureReason}
              </p>
            )}
            <button
              type="button"
              onClick={handleRetryProbe}
              disabled={isLoading}
              className="mt-3 rounded-lg border border-stone-300 dark:border-stone-600 px-3 py-1.5 text-xs text-stone-700 dark:text-stone-200 hover:bg-stone-200 dark:hover:bg-stone-700 disabled:opacity-60">
              {isLoading ? t('keyring.consent.retrying') : t('keyring.settings.retryButton')}
            </button>
          </div>
        </section>

        {/* Consent management (only when keyring is unavailable) */}
        {!keyringStatus.available && (
          <section>
            <h3 className="text-sm font-medium text-stone-700 dark:text-stone-200 mb-3">
              {t('keyring.settings.consentTitle')}
            </h3>
            <p className="text-xs text-stone-500 dark:text-stone-400 mb-3">
              {t('keyring.settings.consentDescription')}
            </p>
            <div className="flex flex-wrap gap-2">
              {keyringStatus.activeMode !== 'local_encrypted' && (
                <button
                  type="button"
                  onClick={() => handleConsentChange('local_encrypted')}
                  disabled={isLoading}
                  className="rounded-lg bg-ocean-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-600 disabled:opacity-60">
                  {t('keyring.settings.grantConsent')}
                </button>
              )}
              {keyringStatus.activeMode !== 'declined' && (
                <button
                  type="button"
                  onClick={() => handleConsentChange('declined')}
                  disabled={isLoading}
                  className="rounded-lg border border-stone-300 dark:border-stone-600 px-3 py-1.5 text-xs text-stone-700 dark:text-stone-200 hover:bg-stone-200 dark:hover:bg-stone-700 disabled:opacity-60">
                  {t('keyring.settings.revokeConsent')}
                </button>
              )}
            </div>
          </section>
        )}

        {error && <p className="text-sm text-coral-400">{error}</p>}
      </div>
    </div>
  );
};

export default SecurityPanel;
