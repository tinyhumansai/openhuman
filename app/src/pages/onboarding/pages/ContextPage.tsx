import * as Sentry from '@sentry/react';
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
        // The completeAndExit chain awaits four core RPCs
        // (app_state_update_local_state → app_state_snapshot →
        // config_set_onboarding_completed → app_state_snapshot). When any
        // rejects, the user sees a silent dead "Continue to chat" button
        // (#2081). The `.catch` below kept the rejection out of Sentry's
        // global handlers because a handled rejection never fires
        // `unhandledrejection`. Forward to Sentry explicitly so the
        // dashboard can show whether #2179 (snapshot timeout) closed the
        // symptom in practice.
        void completeAndExit().catch(error => {
          console.error('[onboarding:context-page] completeAndExit failed', error);
          Sentry.captureException(error, {
            tags: { flow: 'onboarding-complete', step: 'continue-to-chat' },
          });
        });
      }}
      onBack={() => navigate('/onboarding/skills')}
    />
  );
};

export default ContextPage;
