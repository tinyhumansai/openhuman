import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { useOnboardingContext } from '../OnboardingContext';
import RuntimeChoiceStep from '../steps/RuntimeChoiceStep';

const RuntimeChoicePage = () => {
  const navigate = useNavigate();
  const { setDraft, completeAndExit } = useOnboardingContext();
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  useEffect(() => {
    if (isLocalSession) {
      navigate('/onboarding/custom/inference', { replace: true });
    }
  }, [isLocalSession, navigate]);

  if (isLocalSession) {
    return null;
  }

  return (
    <RuntimeChoiceStep
      onNext={async mode => {
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
          console.error('[onboarding:runtime-choice-page] completeAndExit failed', err);
        }
      }}
    />
  );
};

export default RuntimeChoicePage;
