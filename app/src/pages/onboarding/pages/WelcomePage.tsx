import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import WelcomeStep from '../steps/WelcomeStep';

const WelcomePage = () => {
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  useEffect(() => {
    trackEvent('onboarding_start');
    if (isLocalSession) {
      navigate('/onboarding/custom/inference', { replace: true });
    }
  }, [isLocalSession, navigate]);

  if (isLocalSession) {
    return null;
  }

  return (
    <WelcomeStep
      onNext={() => {
        trackEvent('onboarding_step_complete', { step_name: 'welcome' });
        navigate('/onboarding/runtime-choice');
      }}
    />
  );
};

export default WelcomePage;
