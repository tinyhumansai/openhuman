import { trackEvent } from '../../../services/analytics';
import { useOnboardingContext } from '../OnboardingContext';
import ApiKeysStep from '../steps/ApiKeysStep';

const ApiKeysPage = () => {
  const { completeAndExit } = useOnboardingContext();

  const finish = async (skipped: boolean) => {
    trackEvent('onboarding_step_complete', { step_name: 'api_keys', skipped });
    try {
      await completeAndExit();
    } catch (err) {
      console.error('[onboarding:api-keys-page] completeAndExit failed', err);
    }
  };

  return <ApiKeysStep onNext={() => void finish(false)} onSkip={() => void finish(true)} />;
};

export default ApiKeysPage;
