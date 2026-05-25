import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import EmbeddingsPanel from '../../../components/settings/panels/EmbeddingsPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'embeddings' as const;
const STEP_INDEX = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);
const LOCAL_DEFAULT_DISABLED_REASON =
  'Managed setup requires OpenHuman sign-in and is unavailable in local mode.';

const CustomEmbeddingsPage = () => {
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
      testId="onboarding-custom-embeddings-step"
      stepIndex={STEP_INDEX}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t('onboarding.custom.embeddings.title')}
      subtitle={t('onboarding.custom.embeddings.subtitle')}
      defaultDescription={t('onboarding.custom.embeddings.defaultDesc')}
      configureDescription={t('onboarding.custom.embeddings.configureDesc')}
      configureContent={<EmbeddingsPanel embedded />}
      defaultDisabled={isLocalSession}
      defaultDisabledReason={isLocalSession ? LOCAL_DEFAULT_DISABLED_REASON : undefined}
      hideChoiceCards={isLocalSession}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX - 1]])}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: 'custom_embeddings',
          choice: choice ?? 'default',
        });
        if (isLast) {
          try {
            await completeAndExit();
          } catch (err) {
            console.error('[onboarding:custom-embeddings] completeAndExit failed', err);
          }
          return;
        }
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX + 1]]);
      }}
      continueLabel={isLast ? t('onboarding.custom.finish') : undefined}
    />
  );
};

export default CustomEmbeddingsPage;
