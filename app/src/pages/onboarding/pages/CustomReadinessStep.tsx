import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ReadinessPanel from '../../../components/settings/panels/ReadinessPanel';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { CUSTOM_WIZARD_ROUTES, CUSTOM_WIZARD_STEPS } from '../customWizardSteps';
import { useOnboardingContext } from '../OnboardingContext';
import CustomWizardStep from '../steps/CustomWizardStep';

const STEP_KEY = 'readiness' as const;

/**
 * Final onboarding step: run the aggregated readiness checks and confirm the
 * assistant will actually work before finishing setup (issue #4252).
 *
 * The "Finish" action is gated on a validated model connection (acceptance
 * criterion 3). To avoid trapping an offline user, a clearly-labeled,
 * non-silent "skip for now" toggle records the override instead of silently
 * bypassing the gate.
 */
export default function CustomReadinessStep() {
  const { t } = useT();
  const navigate = useNavigate();
  const { completeAndExit } = useOnboardingContext();
  const stepIndex = CUSTOM_WIZARD_STEPS.indexOf(STEP_KEY);

  const [modelOk, setModelOk] = useState(false);
  const [skipOverride, setSkipOverride] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);

  const canFinish = modelOk || skipOverride;

  const configureContent = useMemo(
    () => (
      <div className="flex flex-col gap-4">
        <ReadinessPanel embedded onModelStatusChange={setModelOk} />
        {!modelOk ? (
          <div
            className="rounded-xl border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-4 py-3"
            data-testid="readiness-model-gate">
            <p className="text-xs text-amber-800 dark:text-amber-200 leading-relaxed">
              {t('onboarding.custom.readiness.gateHint')}
            </p>
            <label className="mt-2 flex items-center gap-2 text-xs text-content-secondary">
              <input
                type="checkbox"
                checked={skipOverride}
                onChange={e => setSkipOverride(e.target.checked)}
                data-testid="readiness-skip-toggle"
              />
              {t('onboarding.custom.readiness.skipLabel')}
            </label>
          </div>
        ) : null}
      </div>
    ),
    [modelOk, skipOverride, t]
  );

  return (
    <>
      <CustomWizardStep
        testId="onboarding-custom-readiness-step"
        stepIndex={stepIndex}
        stepCount={CUSTOM_WIZARD_STEPS.length}
        title={t('onboarding.custom.readiness.title')}
        subtitle={t('onboarding.custom.readiness.subtitle')}
        defaultDescription=""
        configureDescription=""
        configureContent={configureContent}
        hideChoiceCards
        choice="configure"
        onChoiceChange={() => {}}
        continueDisabled={!canFinish}
        onBack={() => navigate(CUSTOM_WIZARD_ROUTES[CUSTOM_WIZARD_STEPS[stepIndex - 1]])}
        onContinue={async () => {
          setFinishError(null);
          try {
            await completeAndExit();
            // Only record completion once the exit actually succeeds — a failed
            // attempt must not log a false completion (CR: coderabbitai).
            trackEvent('onboarding_step_complete', {
              step_name: 'custom_readiness',
              model_connection_ok: modelOk,
              skipped_model_gate: skipOverride,
            });
          } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            console.error('[onboarding:custom-readiness] completeAndExit failed', err);
            setFinishError(message);
          }
        }}
        continueLabel={t('onboarding.custom.finish')}
      />
      {finishError ? (
        <div
          className="mt-3 rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300"
          data-testid="onboarding-readiness-exit-error">
          {t('onboarding.custom.readiness.finishError')}
        </div>
      ) : null}
    </>
  );
}
