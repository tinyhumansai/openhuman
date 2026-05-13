import { useNavigate } from 'react-router-dom';

import { trackEvent } from '../../../services/analytics';
import { useOnboardingContext } from '../OnboardingContext';
import ContextGatheringStep from '../steps/ContextGatheringStep';

const ContextPage = () => {
  const navigate = useNavigate();
  const { draft, completeAndExit } = useOnboardingContext();

  return (
    <ContextGatheringStep
      connectedSources={draft.connectedSources}
      // Chat-provider step is disabled for now, so context-gathering is
      // the final step when it runs — finish onboarding directly.
      onNext={() => {
        trackEvent('onboarding_step_complete', { step_name: 'context' });
        void completeAndExit().catch(error => {
          console.error('[onboarding:context-page] completeAndExit failed', error);
        });
      }}
      onBack={() => navigate('/onboarding/skills')}
    />
  );
};

export default ContextPage;
