import type { WalkthroughStepState } from '../../pages/onboarding/OnboardingContext';
import { useWalkthroughUI } from './WalkthroughProvider';

interface WalkthroughActionCardProps {
  step: WalkthroughStepState;
}

/**
 * An action card in the walkthrough flow. Displays the step name, description,
 * and a read-only status marker. The walkthrough previews setup areas; it does
 * not mark integrations or automations as configured without launching those
 * flows.
 */
const WalkthroughActionCard = ({ step }: WalkthroughActionCardProps) => {
  const { stepLabels, stepDescriptions } = useWalkthroughUI();

  const label = stepLabels[step.key] ?? step.key;
  const description = stepDescriptions[step.key] ?? '';

  return (
    <div
      className={`
        w-full text-left p-4 rounded-xl border transition-all duration-200
        flex items-start gap-3
        ${
          step.completed
            ? 'bg-[#2F6EF4]/5 border-[#2F6EF4]/30 cursor-default'
            : 'bg-white dark:bg-neutral-900 border-stone-200 dark:border-neutral-800'
        }
      `}>
      {/* Status indicator */}
      <div
        className={`
          shrink-0 w-6 h-6 rounded-full border-2 flex items-center justify-center
          transition-all duration-300 mt-0.5
          ${
            step.completed
              ? 'bg-[#2F6EF4] border-[#2F6EF4]'
              : 'border-stone-300 dark:border-neutral-600'
          }
        `}>
        {step.completed ? (
          <svg
            className="w-3.5 h-3.5 text-white animate-in zoom-in duration-200"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="3"
            strokeLinecap="round"
            strokeLinejoin="round">
            <polyline points="20 6 9 17 4 12" />
          </svg>
        ) : (
          <span className="h-2 w-2 rounded-full bg-stone-300 dark:bg-neutral-600" />
        )}
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0">
        <h4
          className={`
            text-sm font-semibold transition-colors
            ${step.completed ? 'text-[#2F6EF4]' : 'text-stone-900 dark:text-neutral-100'}
          `}>
          {label}
        </h4>
        {description && (
          <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5 leading-relaxed">
            {description}
          </p>
        )}
      </div>
    </div>
  );
};

export default WalkthroughActionCard;
