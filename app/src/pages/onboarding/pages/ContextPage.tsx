import { useNavigate } from 'react-router-dom';

import { useOnboardingContext } from '../OnboardingContext';
import ContextGatheringStep from '../steps/ContextGatheringStep';

const ContextPage = () => {
  const navigate = useNavigate();
  const { draft } = useOnboardingContext();

  return (
    <ContextGatheringStep
      connectedSources={draft.connectedSources}
      onNext={() => navigate('/onboarding/chat-provider')}
      onBack={() => navigate('/onboarding/skills')}
    />
  );
};

export default ContextPage;
