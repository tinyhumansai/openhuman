import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import VoicePanel from '../../../components/settings/panels/VoicePanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'voice' as const;
const STEP_INDEX = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);

const CustomVoicePage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { draft, setDraft, completeAndExit } = useOnboardingContext();

  const [choice, setChoice] = useState<CustomStepChoice | null>(
    draft.customChoices?.[STEP_KEY] ?? null
  );

  const persistChoice = (next: CustomStepChoice) => {
    setChoice(next);
    setDraft(prev => ({ ...prev, customChoices: { ...prev.customChoices, [STEP_KEY]: next } }));
  };

  const isLast = STEP_INDEX === CUSTOM_WIZARD_STEPS.length - 1;

  return (
    <CustomWizardStep
      testId="onboarding-custom-voice-step"
      stepIndex={STEP_INDEX}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t('onboarding.custom.voice.title')}
      subtitle={t('onboarding.custom.voice.subtitle')}
      defaultDescription={t('onboarding.custom.voice.defaultDesc')}
      configureDescription={t('onboarding.custom.voice.configureDesc')}
      configureContent={<VoicePanel embedded />}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX - 1]])}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: 'custom_voice',
          choice: choice ?? 'default',
        });
        if (isLast) {
          try {
            await completeAndExit();
          } catch (err) {
            console.error('[onboarding:custom-voice] completeAndExit failed', err);
          }
          return;
        }
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX + 1]]);
      }}
      continueLabel={isLast ? t('onboarding.custom.finish') : undefined}
    />
  );
};

export default CustomVoicePage;
