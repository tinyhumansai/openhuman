import { useNavigate } from 'react-router-dom';

import { trackEvent } from '../../../services/analytics';
import { useOnboardingContext } from '../OnboardingContext';
import RuntimeChoiceStep from '../steps/RuntimeChoiceStep';

const RuntimeChoicePage = () => {
  const navigate = useNavigate();
  const { setDraft, completeAndExit } = useOnboardingContext();

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
