import { useT } from '../../../lib/i18n/I18nContext';

interface WizardStepperProps {
  /** Ordered labels for each step in the wizard. */
  labels: string[];
  /** Zero-based index of the step that is currently active. */
  activeIndex: number;
}

/**
 * Horizontal step indicator rendered above the body of each custom-wizard
 * step. Renders a dot per step, connected by lines, with three visual
 * states: completed (filled sage with a check), active (filled primary),
 * and upcoming (outlined stone).
 */
const WizardStepper = ({ labels, activeIndex }: WizardStepperProps) => {
  const { t } = useT();
  return (
    <ol
      role="list"
      aria-label={t('onboarding.custom.progressAriaLabel')}
      className="flex w-full items-start justify-between"
      data-testid="onboarding-wizard-stepper">
      {labels.map((label, idx) => {
        const completed = idx < activeIndex;
        const active = idx === activeIndex;
        const isLast = idx === labels.length - 1;

        const dotClasses = completed
          ? 'bg-sage-500 border-sage-500 text-white'
          : active
            ? 'bg-primary-500 border-primary-500 text-white'
            : 'bg-white border-stone-300 text-stone-400';

        const labelClasses = completed
          ? 'text-sage-700'
          : active
            ? 'text-stone-900 font-semibold'
            : 'text-stone-400';

        const connectorClasses = completed ? 'bg-sage-500' : 'bg-stone-200';

        return (
          <li
            key={label}
            aria-current={active ? 'step' : undefined}
            className="relative flex flex-1 flex-col items-center">
            <div className="flex w-full items-center">
              {/* Left spacer / connector */}
              <div
                aria-hidden
                className={`h-0.5 flex-1 ${idx === 0 ? 'opacity-0' : connectorClasses}`}
              />
              <div
                className={`flex h-6 w-6 flex-none items-center justify-center rounded-full border-2 text-[10px] font-semibold ${dotClasses}`}>
                {completed ? (
                  <svg
                    viewBox="0 0 12 12"
                    className="h-3 w-3"
                    fill="none"
                    aria-hidden
                    xmlns="http://www.w3.org/2000/svg">
                    <path
                      d="M2 6.5L5 9.5L10 3.5"
                      stroke="currentColor"
                      strokeWidth="1.75"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                ) : (
                  idx + 1
                )}
              </div>
              {/* Right connector */}
              <div
                aria-hidden
                className={`h-0.5 flex-1 ${isLast ? 'opacity-0' : connectorClasses}`}
              />
            </div>
            <span className={`mt-2 text-[11px] leading-tight text-center ${labelClasses}`}>
              {label}
            </span>
          </li>
        );
      })}
    </ol>
  );
};

export default WizardStepper;
