import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ComposioPanel from '../../../components/settings/panels/ComposioPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'oauth' as const;
const STEP_INDEX = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);
const LOCAL_DEFAULT_DISABLED_REASON =
  'Managed setup requires OpenHuman sign-in and is unavailable in local mode.';

const CustomOAuthPage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const { draft, setDraft, completeAndExit } = useOnboardingContext();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  const [choice, setChoice] = useState<CustomStepChoice | null>(
    draft.customChoices?.[STEP_KEY] ?? (isLocalSession ? 'configure' : null)
  );

  useEffect(() => {
    if (!isLocalSession) {
      return;
    }
    setChoice('configure');
    setDraft(prev => ({
      ...prev,
      customChoices: { ...prev.customChoices, [STEP_KEY]: 'configure' },
    }));
  }, [isLocalSession, setDraft]);

  const persistChoice = (next: CustomStepChoice) => {
    setChoice(next);
    setDraft(prev => ({ ...prev, customChoices: { ...prev.customChoices, [STEP_KEY]: next } }));
  };

  const isLast = STEP_INDEX === CUSTOM_WIZARD_STEPS.length - 1;

  return (
    <CustomWizardStep
      testId="onboarding-custom-oauth-step"
      stepIndex={STEP_INDEX}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t('onboarding.custom.oauth.title')}
      subtitle={t('onboarding.custom.oauth.subtitle')}
      defaultDescription={t('onboarding.custom.oauth.defaultDesc')}
      configureDescription={t('onboarding.custom.oauth.configureDesc')}
      configureContent={<ComposioPanel embedded managedAuthEnabled={false} />}
      defaultDisabled={isLocalSession}
      defaultDisabledReason={isLocalSession ? LOCAL_DEFAULT_DISABLED_REASON : undefined}
      hideChoiceCards={isLocalSession}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX - 1]])}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: 'custom_oauth',
          choice: choice ?? 'default',
        });
        if (isLast) {
          try {
            await completeAndExit();
          } catch (err) {
            console.error('[onboarding:custom-oauth] completeAndExit failed', err);
          }
          return;
        }
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX + 1]]);
      }}
      continueLabel={isLast ? t('onboarding.custom.finish') : undefined}
    />
  );
};

export default CustomOAuthPage;
