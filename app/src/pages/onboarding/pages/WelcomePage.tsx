import { useNavigate } from 'react-router-dom';

import WelcomeStep from '../steps/WelcomeStep';

const WelcomePage = () => {
  const navigate = useNavigate();
  return <WelcomeStep onNext={() => navigate('/onboarding/skills')} />;
};

export default WelcomePage;
