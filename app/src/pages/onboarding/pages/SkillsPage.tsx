import { useNavigate } from 'react-router-dom';

import { useOnboardingContext } from '../OnboardingContext';
import SkillsStep, { type SkillsConnections } from '../steps/SkillsStep';

const SkillsPage = () => {
  const navigate = useNavigate();
  const { setDraft, completeAndExit } = useOnboardingContext();

  const handleNext = async ({ sources }: SkillsConnections) => {
    console.debug('[onboarding:skills-page] next', { sources });
    setDraft(prev => ({ ...prev, connectedSources: sources }));

    // Route to ContextGatheringStep when there's a Composio source the
    // pipeline can drive. Otherwise jump straight to onboarding completion.
    const hasComposioSource = sources.some(s => s.startsWith('composio:'));
    if (hasComposioSource) {
      navigate('/onboarding/context');
    } else {
      await completeAndExit();
    }
  };

  return <SkillsStep onNext={handleNext} onBack={() => navigate('/onboarding/welcome')} />;
};

export default SkillsPage;
