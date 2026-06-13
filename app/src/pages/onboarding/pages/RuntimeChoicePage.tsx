import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { useOnboardingContext } from '../OnboardingContext';
import RuntimeChoiceStep from '../steps/RuntimeChoiceStep';

const RuntimeChoicePage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { setDraft, completeAndExit } = useOnboardingContext();
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);
  const [exitError, setExitError] = useState<string | null>(null);

  useEffect(() => {
    if (isLocalSession) {
      navigate('/onboarding/custom/inference', { replace: true });
    }
  }, [isLocalSession, navigate]);

  if (isLocalSession) {
    return null;
  }

  return (
    <>
      <RuntimeChoiceStep
        onNext={async mode => {
          setExitError(null);
          setDraft(prev => ({ ...prev, aiMode: mode }));
          trackEvent('onboarding_step_complete', { step_name: 'runtime_choice', ai_mode: mode });

          if (mode === 'custom') {
            navigate('/onboarding/custom/inference');
            return;
          }
          // Cloud path: nothing else to configure, finish onboarding.
          try {
            await completeAndExit();
          } catch (err) {
            const safeErr = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
            console.error('[onboarding:runtime-choice-page] completeAndExit failed', safeErr);
            setExitError(err instanceof Error ? err.message : String(err));
          }
        }}
      />
      {exitError ? (
        <div
          className="mt-3 rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300"
          data-testid="onboarding-runtime-choice-exit-error">
          {t('onboarding.runtimeChoice.exitError')}
        </div>
      ) : null}
    </>
  );
};

export default RuntimeChoicePage;
