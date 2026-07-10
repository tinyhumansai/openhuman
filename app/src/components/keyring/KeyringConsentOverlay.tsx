import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { decideKeyringConsent, retryKeyringProbe } from '../../services/keyringApi';

const KeyringConsentOverlay = () => {
  const { t } = useT();
  const { snapshot } = useCoreState();
  const [isRetrying, setIsRetrying] = useState(false);
  const [isConsenting, setIsConsenting] = useState(false);
  const [showDetails, setShowDetails] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const keyringStatus = snapshot.keyringStatus;
  const needsConsent = keyringStatus.activeMode === 'consent_pending';

  if (!needsConsent) {
    return null;
  }

  const handleConsent = async () => {
    setIsConsenting(true);
    setError(null);
    try {
      await decideKeyringConsent('local_encrypted');
    } catch {
      setError(t('keyring.consent.error'));
    } finally {
      setIsConsenting(false);
    }
  };

  const handleDecline = async () => {
    setIsConsenting(true);
    setError(null);
    try {
      await decideKeyringConsent('declined');
    } catch {
      setError(t('keyring.consent.error'));
    } finally {
      setIsConsenting(false);
    }
  };

  const handleRetry = async () => {
    setIsRetrying(true);
    setError(null);
    try {
      await retryKeyringProbe();
    } catch {
      setError(t('keyring.consent.retryFailed'));
    } finally {
      setIsRetrying(false);
    }
  };

  const failureReason = keyringStatus.failureReason;

  return (
    <div className="fixed inset-0 z-[10000] bg-stone-950/80 backdrop-blur-sm flex items-center justify-center p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="keyring-consent-title"
        className="w-full max-w-lg rounded-2xl border border-amber-500/30 bg-stone-900 p-6 shadow-2xl">
        <div className="flex items-center gap-3 mb-4">
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-amber-500/20">
            <svg
              className="h-5 w-5 text-amber-400"
              fill="none"
              viewBox="0 0 24 24"
              strokeWidth={1.5}
              stroke="currentColor">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M16.5 10.5V6.75a4.5 4.5 0 1 0-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 0 0 2.25-2.25v-6.75a2.25 2.25 0 0 0-2.25-2.25H6.75a2.25 2.25 0 0 0-2.25 2.25v6.75a2.25 2.25 0 0 0 2.25 2.25Z"
              />
            </svg>
          </div>
          <h2 id="keyring-consent-title" className="text-lg font-semibold text-white">
            {t('keyring.consent.title')}
          </h2>
        </div>

        <p className="text-sm text-content-faint">{t('keyring.consent.description')}</p>

        {failureReason && (
          <p className="mt-2 text-xs text-content-faint">
            {t('keyring.consent.reasonPrefix')} {failureReason}
          </p>
        )}

        <button
          type="button"
          onClick={() => setShowDetails(!showDetails)}
          className="mt-3 text-xs text-ocean-400 hover:text-ocean-300 underline">
          {showDetails ? t('keyring.consent.hideDetails') : t('keyring.consent.showDetails')}
        </button>

        {showDetails && (
          <div className="mt-2 rounded-lg bg-stone-800/60 p-3 text-xs text-content-faint leading-relaxed">
            <p className="font-medium text-content-faint mb-1">
              {t('keyring.consent.tradeoffTitle')}
            </p>
            <p>{t('keyring.consent.tradeoffBody')}</p>
          </div>
        )}

        {error && <p className="mt-3 text-sm text-coral-300">{error}</p>}

        <div className="mt-5 flex flex-wrap gap-3">
          <button
            type="button"
            onClick={handleConsent}
            disabled={isConsenting || isRetrying}
            className="rounded-lg bg-ocean-500 px-4 py-2 text-sm font-medium text-white hover:bg-ocean-600 disabled:opacity-60">
            {isConsenting ? t('common.loading') : t('keyring.consent.consentButton')}
          </button>
          <button
            type="button"
            onClick={handleRetry}
            disabled={isRetrying || isConsenting}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
            {isRetrying ? t('keyring.consent.retrying') : t('keyring.consent.retryButton')}
          </button>
          <button
            type="button"
            onClick={handleDecline}
            disabled={isConsenting || isRetrying}
            className="rounded-lg border border-stone-700 px-4 py-2 text-sm text-content-faint hover:bg-stone-800 disabled:opacity-60">
            {t('keyring.consent.declineButton')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default KeyringConsentOverlay;
