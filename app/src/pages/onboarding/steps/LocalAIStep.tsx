import { useCallback, useEffect, useRef, useState } from 'react';

import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { bootstrapLocalAiWithRecommendedPreset } from '../../../utils/localAiBootstrap';
import { openhumanLocalAiPresets } from '../../../utils/tauriCommands';
import OnboardingNextButton from '../components/OnboardingNextButton';

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
  onBack?: () => void;
  onDownloadError?: (message: string) => void;
}

const LocalAIStep = ({ onNext, onBack: _onBack, onDownloadError }: LocalAIStepProps) => {
  const { t } = useT();
  const downloadStartedRef = useRef(false);
  const [recommendDisabled, setRecommendDisabled] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    // Read-only probe: never apply/persist a preset from the mount effect.
    // Preset application lives in handleConsent via bootstrapLocalAiWithRecommendedPreset.
    openhumanLocalAiPresets()
      .then(presets => {
        if (!cancelled) {
          setRecommendDisabled(presets.recommend_disabled ?? false);
        }
      })
      .catch(() => {
        if (!cancelled) setRecommendDisabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleConsent = useCallback(() => {
    if (downloadStartedRef.current) return;
    downloadStartedRef.current = true;
    console.debug('[LocalAIStep] starting background Local AI bootstrap after consent');

    // Fire-and-forget: start bootstrap in the background — the global snackbar tracks progress.
    void bootstrapLocalAiWithRecommendedPreset(false, '[LocalAIStep]').catch((err: unknown) => {
      console.warn('[LocalAIStep] Local AI bootstrap failed:', err);
      onDownloadError?.(t('onboarding.localAI.setupIssue'));
    });

    // Advance to next step immediately
    onNext({ consentGiven: true, downloadStarted: true });
  }, [onNext, onDownloadError, t]);

  const handleSkip = useCallback(() => {
    console.debug('[LocalAIStep] skipping local AI — using cloud fallback');
    onNext({ consentGiven: false, downloadStarted: false });
  }, [onNext]);

  // Still probing device — show nothing yet.
  if (recommendDisabled === null) {
    return null;
  }

  // Low-RAM device: show cloud fallback option as the primary path.
  if (recommendDisabled) {
    return (
      <div className="rounded-2xl border border-line bg-surface p-8 shadow-soft animate-fade-up">
        <div className="flex flex-col items-center mb-5">
          <div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary-50 dark:bg-primary-500/15 mb-3">
            <svg
              className="h-8 w-8 text-primary-500"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M2.25 15a4.5 4.5 0 004.5 4.5H18a3.75 3.75 0 001.332-7.257 3 3 0 00-3.758-3.848 5.25 5.25 0 00-10.233 2.33A4.502 4.502 0 002.25 15z"
              />
            </svg>
          </div>
          <h1 className="text-xl font-bold mb-2 text-content">{t('onboarding.localAI')}</h1>
          <p className="text-content-secondary text-sm text-center">
            {t('onboarding.localAIDesc')}
          </p>
        </div>

        <div className="space-y-2 mb-5">
          <div className="rounded-xl border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/15 px-3 py-2">
            <p className="text-xs text-content-secondary">
              <span className="font-semibold">{t('onboarding.localAI')}</span>
              <span className="text-content-secondary">&nbsp;— {t('onboarding.localAIDesc')}</span>
            </p>
          </div>
          <div className="rounded-xl border border-line bg-surface-muted px-3 py-2">
            <p className="text-xs text-content-secondary">
              <span className="font-semibold">{t('common.download')}</span>
              <span className="text-content-secondary">&nbsp;— {t('misc.downloading')}</span>
            </p>
          </div>
          <div className="rounded-xl border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2">
            <p className="text-xs text-content-secondary">
              <span className="font-semibold">{t('welcome.connect')}</span>
              <span className="text-content-secondary">&nbsp;— {t('onboarding.localAIDesc')}</span>
            </p>
          </div>
        </div>

        <OnboardingNextButton
          label={t('onboarding.localAI.continueWithCloud')}
          onClick={handleSkip}
        />

        <Button variant="tertiary" size="xs" onClick={handleConsent} className="mt-3 w-full">
          {t('onboarding.localAI.useLocalAnyway')}
        </Button>
      </div>
    );
  }

  // Sufficient RAM: local AI is opt-in. Present cloud as the primary path and
  // local AI as an explicit choice for users who want full privacy.
  return (
    <div className="rounded-2xl border border-line bg-surface p-8 shadow-soft animate-fade-up">
      <div className="flex flex-col items-center mb-5">
        <div className="flex h-16 w-16 items-center justify-center rounded-full bg-primary-50 dark:bg-primary-500/15 mb-3">
          <svg
            className="h-8 w-8 text-primary-500"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={1.5}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M2.25 15a4.5 4.5 0 004.5 4.5H18a3.75 3.75 0 001.332-7.257 3 3 0 00-3.758-3.848 5.25 5.25 0 00-10.233 2.33A4.502 4.502 0 002.25 15z"
            />
          </svg>
        </div>
        <h1 className="text-xl font-bold mb-2 text-content">{t('onboarding.localAI')}</h1>
        <p className="text-content-secondary text-sm text-center">{t('onboarding.localAIDesc')}</p>
      </div>

      <div className="space-y-2 mb-5">
        <div className="rounded-xl border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/15 px-3 py-2">
          <p className="text-xs text-content-secondary">
            <span className="font-semibold">{t('onboarding.localAI')}</span>
            <span className="text-content-secondary">&nbsp;— {t('onboarding.localAIDesc')}</span>
          </p>
        </div>
        <div className="rounded-xl border border-line bg-surface-muted px-3 py-2">
          <p className="text-xs text-content-secondary">
            <span className="font-semibold">{t('onboarding.localAI')}</span>
            <span className="text-content-secondary">&nbsp;— {t('onboarding.localAIDesc')}</span>
          </p>
        </div>
        <div className="rounded-xl border border-line bg-surface-muted px-3 py-2">
          <p className="text-xs text-content-secondary">
            <span className="font-semibold">{t('common.refresh')}</span>
            <span className="text-content-secondary">&nbsp;— {t('onboarding.localAIDesc')}</span>
          </p>
        </div>
      </div>

      <OnboardingNextButton
        label={t('onboarding.localAI.continueWithCloud')}
        onClick={handleSkip}
      />

      <Button variant="tertiary" size="xs" onClick={handleConsent} className="mt-3 w-full">
        {t('onboarding.localAI.useLocalInstead')}
      </Button>
    </div>
  );
};

export default LocalAIStep;
