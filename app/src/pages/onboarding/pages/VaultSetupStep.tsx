import { useCallback, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import MemoryDataPanel from '../../../components/settings/panels/MemoryDataPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { type CustomStepChoice, useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'vault' as const;

export default function VaultSetupStep() {
  const { t } = useT();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const { draft, setDraft } = useOnboardingContext();
  const stepIndex = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  const appliedLocalRef = useRef(false);
  const initialChoice = isLocalSession ? 'configure' : (draft.customChoices?.[STEP_KEY] ?? null);
  const [choice, setChoice] = useState<CustomStepChoice | null>(initialChoice);

  if (isLocalSession && !appliedLocalRef.current) {
    appliedLocalRef.current = true;
    if (choice !== 'configure') {
      setChoice('configure');
    }
    setDraft(prev => ({
      ...prev,
      customChoices: { ...prev.customChoices, [STEP_KEY]: 'configure' },
    }));
  }

  const persistChoice = useCallback(
    (next: CustomStepChoice) => {
      setChoice(next);
      setDraft(prev => ({ ...prev, customChoices: { ...prev.customChoices, [STEP_KEY]: next } }));
    },
    [setDraft]
  );

  const configureContent = useMemo(() => <MemoryDataPanel embedded />, []);

  return (
    <>
      <CustomWizardStep
        testId="onboarding-custom-vault-step"
        stepIndex={stepIndex}
        stepCount={CUSTOM_WIZARD_STEPS.length}
        title={t('onboarding.custom.vault.title')}
        subtitle={t('onboarding.custom.vault.subtitle')}
        defaultDescription={t('onboarding.custom.vault.defaultDesc')}
        configureDescription={t('onboarding.custom.vault.configureDesc')}
        configureContent={configureContent}
        defaultDisabled={isLocalSession}
        defaultDisabledReason={
          isLocalSession ? t('onboarding.custom.vault.localDisabledReason') : undefined
        }
        hideChoiceCards={isLocalSession}
        choice={choice}
        onChoiceChange={persistChoice}
        onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex - 1]])}
        onContinue={() => {
          trackEvent('onboarding_step_complete', {
            step_name: 'custom_vault',
            choice: choice ?? 'default',
          });
          navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex + 1]]);
        }}
        continueLabel={t('onboarding.custom.continue')}
      />
    </>
  );
}
