import { useNavigate } from 'react-router-dom';

import { useOnboardingContext } from '../OnboardingContext';
import SkillsStep from '../steps/SkillsStep';

const SkillsPage = () => {
  const navigate = useNavigate();
  const { setDraft } = useOnboardingContext();

  const handleNext = (connectedSources: string[]) => {
    console.debug('[onboarding:skills-page] next', { connectedSources });
    setDraft(prev => ({ ...prev, connectedSources }));
    if (connectedSources.length === 0) {
      // No sources connected — skip context gathering.
      navigate('/onboarding/chat-provider');
    } else {
      navigate('/onboarding/context');
    }
  };

  return <SkillsStep onNext={handleNext} onBack={() => navigate('/onboarding/welcome')} />;
};

export default SkillsPage;
