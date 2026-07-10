import { useT } from '../../lib/i18n/I18nContext';
import type { HarnessInitSnapshot, HarnessInitStep } from '../../services/harnessInitService';
import { CheckIcon, CloseIcon, Spinner } from '../ui/icons';

/** Map a known step id to its i18n label key; fall back to the server label. */
function stepLabel(t: (key: string) => string, step: HarnessInitStep): string {
  switch (step.id) {
    case 'python_runtime':
      return t('harnessInit.stepPython');
    case 'spacy':
      return t('harnessInit.stepSpacy');
    case 'node_runtime':
      return t('harnessInit.stepNode');
    default:
      return step.label;
  }
}

function StepRow({ step }: { step: HarnessInitStep }) {
  const { t } = useT();
  const label = stepLabel(t, step);

  let icon: React.ReactNode;
  let stateText: string;
  switch (step.state) {
    case 'running':
      icon = <Spinner className="h-4 w-4 text-primary-500" />;
      stateText = t('harnessInit.stateRunning');
      break;
    case 'done':
      icon = <CheckIcon className="h-4 w-4 text-sage-500" />;
      stateText = t('harnessInit.stateDone');
      break;
    case 'failed':
      icon = <CloseIcon className="h-4 w-4 text-coral-400" />;
      stateText = t('harnessInit.stateFailed');
      break;
    case 'skipped':
      icon = <CloseIcon className="h-4 w-4 text-amber-400" />;
      stateText = t('harnessInit.stateSkipped');
      break;
    case 'pending':
    default:
      icon = <span className="block h-2 w-2 rounded-full bg-stone-500/60" />;
      stateText = t('harnessInit.statePending');
      break;
  }

  return (
    <li className="flex items-center gap-3 py-2">
      <span className="flex h-5 w-5 items-center justify-center" aria-hidden>
        {icon}
      </span>
      <span className="flex-1 text-sm text-stone-200">{label}</span>
      <span className="text-xs text-content-faint">{stateText}</span>
    </li>
  );
}

export interface InitProgressScreenProps {
  snapshot: HarnessInitSnapshot;
  onRetry: () => void;
  onContinue: () => void;
  retrying?: boolean;
}

/**
 * Presentational first-run initialization screen. Renders the step list with
 * per-step status. On a terminal `failed` overall it surfaces the failing step
 * and offers Retry / Continue (failures are non-fatal — the app degrades to a
 * fallback).
 */
export default function InitProgressScreen({
  snapshot,
  onRetry,
  onContinue,
  retrying = false,
}: InitProgressScreenProps) {
  const { t } = useT();
  const failed = snapshot.overall === 'failed';
  const failedStep = snapshot.steps.find(s => s.state === 'failed');

  return (
    <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-stone-950/90 p-4 backdrop-blur-sm">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="harness-init-title"
        className="w-full max-w-md rounded-2xl border border-stone-700/60 bg-stone-900 p-6 shadow-2xl">
        <h2 id="harness-init-title" className="text-lg font-semibold text-white">
          {t('harnessInit.title')}
        </h2>
        <p className="mt-1 text-sm text-content-faint">{t('harnessInit.subtitle')}</p>

        <ul className="mt-5 divide-y divide-stone-800">
          {snapshot.steps.map(step => (
            <StepRow key={step.id} step={step} />
          ))}
        </ul>

        {!failed && (
          <div className="mt-5 flex items-center justify-between gap-3">
            <p className="text-xs text-content-muted">{t('harnessInit.backgroundHint')}</p>
            <button
              type="button"
              onClick={onContinue}
              className="shrink-0 rounded-lg border border-stone-700 px-3 py-1.5 text-sm text-content-faint hover:bg-stone-800 hover:text-white">
              {t('harnessInit.runInBackground')}
            </button>
          </div>
        )}

        {failed && (
          <div className="mt-5">
            <div className="rounded-xl border border-coral-500/20 bg-coral-500/10 p-3">
              <p className="text-xs text-coral-300">{t('harnessInit.failedMessage')}</p>
              {failedStep?.message && (
                <p className="mt-1 break-words text-[11px] text-coral-400/80">
                  {failedStep.message}
                </p>
              )}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={onContinue}
                className="rounded-lg px-3 py-1.5 text-sm text-content-faint hover:text-white">
                {t('harnessInit.continueAnyway')}
              </button>
              <button
                type="button"
                onClick={onRetry}
                disabled={retrying}
                className="inline-flex items-center gap-2 rounded-lg bg-primary-500 px-3 py-1.5 text-sm font-medium text-content-inverted hover:bg-primary-500/90 disabled:opacity-60">
                {retrying && <Spinner className="h-3 w-3" />}
                {t('harnessInit.retry')}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
