import createDebug from 'debug';
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

const log = createDebug('app:onboarding:custom');

const describeError = (err: unknown): string => (err instanceof Error ? err.message : String(err));

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
  const { snapshot, clearSession } = useCoreState();
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
  const isFirst = stepIndex === 0;
  const namespace = `onboarding.custom.${stepKey}`;

  const handleBack = async () => {
    // Going back from the first step returns to the welcome/login screen.
    // A session is always present here (OAuth or "Continue Locally"), so we
    // must clear it first — otherwise PublicRoute bounces "/" to /home.
    if (isFirst) {
      try {
        await clearSession();
      } catch (err) {
        // Navigating to "/" with a live session would just bounce back to /home
        // via PublicRoute — so stay on the step and surface a dev-only diagnostic.
        log('[onboarding:custom-%s] clearSession on back failed: %s', stepKey, describeError(err));
        return;
      }
    }
    navigate(backRoute ?? CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex - 1]]);
  };

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
      onBack={() => void handleBack()}
      onContinue={async () => {
        trackEvent('onboarding_step_complete', {
          step_name: `custom_${stepKey}`,
          choice: choice ?? 'default',
        });
        if (isLast) {
          try {
            await completeAndExit();
          } catch (err) {
            log('[onboarding:custom-%s] completeAndExit failed: %s', stepKey, describeError(err));
          }
          return;
        }
        navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex + 1]]);
      }}
      continueLabel={isLast ? t('onboarding.custom.finish') : undefined}
    />
  );
}
