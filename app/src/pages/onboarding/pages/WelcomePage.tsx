import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import { trackEvent } from '../../../services/analytics';
import WelcomeStep from '../steps/WelcomeStep';

const WelcomePage = () => {
  const navigate = useNavigate();

  useEffect(() => {
    trackEvent('onboarding_start');
  }, []);

  return (
    <WelcomeStep
      onNext={() => {
        trackEvent('onboarding_step_complete', { step_name: 'welcome' });
        navigate('/onboarding/skills');
      }}
    />
  );
};

export default WelcomePage;
