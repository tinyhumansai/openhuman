import { useEffect, useMemo, useState } from 'react';
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
const LOCAL_DEFAULT_DISABLED_REASON =
  'Managed setup requires OpenHuman sign-in and is unavailable in local mode.';

export default function VaultSetupStep() {
  const { t } = useT();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const { draft, setDraft, completeAndExit } = useOnboardingContext();
  const stepIndex = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  const [choice, setChoice] = useState<CustomStepChoice | null>(
    draft.customChoices?.[STEP_KEY] ?? (isLocalSession ? 'configure' : null)
  );

  useEffect(() => {
    if (!isLocalSession) return;
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

  const configureContent = useMemo(() => <MemoryDataPanel embedded />, []);

  return (
    <CustomWizardStep
      testId="onboarding-custom-vault-step"
      stepIndex={stepIndex}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title="Memory & Vault Setup"
      subtitle="Confirm where memory notes are written, how source data is read, and whether your vault pipeline is healthy."
      defaultDescription="Use OpenHuman-managed memory defaults. Vault path and sync health can still be reviewed later."
      configureDescription="Review vault ownership, run health checks, and tune memory controls now."
      configureContent={configureContent}
      defaultDisabled={isLocalSession}
      defaultDisabledReason={isLocalSession ? LOCAL_DEFAULT_DISABLED_REASON : undefined}
      hideChoiceCards={isLocalSession}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex - 1]])}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: 'custom_vault',
          choice: choice ?? 'default',
        });
        try {
          await completeAndExit();
        } catch (err) {
          console.error('[onboarding:custom-vault] completeAndExit failed', err);
        }
      }}
      continueLabel={t('onboarding.custom.finish')}
    />
  );
}
