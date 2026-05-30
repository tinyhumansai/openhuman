import { type ReactNode, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { trackEvent } from '../../../services/analytics';
import { isLocalSessionToken } from '../../../utils/localSession';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import {
  type CustomStepChoice,
  type CustomStepKey,
  useOnboardingContext,
} from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const LOCAL_DEFAULT_DISABLED_REASON =
  'Managed setup requires OpenHuman sign-in and is unavailable in local mode.';

interface CustomWizardConfigPageProps {
  stepKey: CustomStepKey;
  configureContent: ReactNode;
  backRoute?: string;
}

export default function CustomWizardConfigPage({
  stepKey,
  configureContent,
  backRoute,
}: CustomWizardConfigPageProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const { draft, setDraft, completeAndExit } = useOnboardingContext();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);
  const stepIndex = CUSTOM_WIZARD_STEPS.indexOf(stepKey);
  const [choice, setChoice] = useState<CustomStepChoice | null>(
    draft.customChoices?.[stepKey] ?? (isLocalSession ? 'configure' : null)
  );

  useEffect(() => {
    if (!isLocalSession) return;
    setChoice('configure');
    setDraft(prev => ({
      ...prev,
      customChoices: { ...prev.customChoices, [stepKey]: 'configure' },
    }));
  }, [isLocalSession, setDraft, stepKey]);

  const persistChoice = (next: CustomStepChoice) => {
    setChoice(next);
    setDraft(prev => ({ ...prev, customChoices: { ...prev.customChoices, [stepKey]: next } }));
  };

  const isLast = stepIndex === CUSTOM_WIZARD_STEPS.length - 1;
  const namespace = `onboarding.custom.${stepKey}`;

  return (
    <CustomWizardStep
      testId={`onboarding-custom-${stepKey}-step`}
      stepIndex={stepIndex}
      stepCount={CUSTOM_WIZARD_STEPS.length}
      title={t(`${namespace}.title`)}
      subtitle={t(`${namespace}.subtitle`)}
      defaultDescription={t(`${namespace}.defaultDesc`)}
      configureDescription={t(`${namespace}.configureDesc`)}
      configureContent={configureContent}
      defaultDisabled={isLocalSession}
      defaultDisabledReason={isLocalSession ? LOCAL_DEFAULT_DISABLED_REASON : undefined}
      hideChoiceCards={isLocalSession}
      choice={choice}
      onChoiceChange={persistChoice}
      onBack={() => navigate(backRoute ?? CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex - 1]])}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: `custom_${stepKey}`,
          choice: choice ?? 'default',
        });
        if (isLast) {
          try {
            await completeAndExit();
          } catch (err) {
            console.error(`[onboarding:custom-${stepKey}] completeAndExit failed`, err);
          }
          return;
        }
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex + 1]]);
      }}
      continueLabel={isLast ? t('onboarding.custom.finish') : undefined}
    />
  );
}
