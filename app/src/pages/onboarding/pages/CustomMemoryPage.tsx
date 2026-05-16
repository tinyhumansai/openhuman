import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import MemoryDataPanel from '../../../components/settings/panels/MemoryDataPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'memory' as const;
const STEP_INDEX = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);

const CustomMemoryPage = () => {
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

  const handleFinish = async () => {
    trackEvent('onboarding_step_complete', {
      step_name: 'custom_memory',
      choice: choice ?? 'default',
    });
    try {
      await completeAndExit();
    } catch (err) {
      console.error('[onboarding:custom-memory] completeAndExit failed', err);
    }
  };

  return (
    <CustomWizardStep
      testId="onboarding-custom-memory-step"
      stepIndex={STEP_INDEX}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t('onboarding.custom.memory.title')}
      subtitle={t('onboarding.custom.memory.subtitle')}
      defaultDescription={t('onboarding.custom.memory.defaultDesc')}
      configureDescription={t('onboarding.custom.memory.configureDesc')}
      configureContent={<MemoryDataPanel embedded />}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX - 1]])}
      onContinue={() => void handleFinish()}
      continueLabel={t('onboarding.custom.finish')}
    />
  );
};

export default CustomMemoryPage;
