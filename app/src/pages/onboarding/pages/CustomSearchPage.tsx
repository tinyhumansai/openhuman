import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ToolsPanel from '../../../components/settings/panels/ToolsPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'search' as const;
const STEP_INDEX = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);

const CustomSearchPage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { draft, setDraft } = useOnboardingContext();

  const [choice, setChoice] = useState<CustomStepChoice | null>(
    draft.customChoices?.[STEP_KEY] ?? null
  );

  const persistChoice = (next: CustomStepChoice) => {
    setChoice(next);
    setDraft(prev => ({ ...prev, customChoices: { ...prev.customChoices, [STEP_KEY]: next } }));
  };

  return (
    <CustomWizardStep
      testId="onboarding-custom-search-step"
      stepIndex={STEP_INDEX}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t('onboarding.custom.search.title')}
      subtitle={t('onboarding.custom.search.subtitle')}
      defaultDescription={t('onboarding.custom.search.defaultDesc')}
      configureDescription={t('onboarding.custom.search.configureDesc')}
      configureContent={<ToolsPanel embedded />}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX - 1]])}
      onContinue={() => {
        trackEvent('onboarding_step_complete', {
          step_name: 'custom_search',
          choice: choice ?? 'default',
        });
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[STEP_INDEX + 1]]);
      }}
    />
  );
};

export default CustomSearchPage;
