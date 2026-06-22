import { useT } from '../../lib/i18n/I18nContext';
import type { WalkthroughPhase } from '../../pages/onboarding/OnboardingContext';
import { useWalkthroughUI } from './WalkthroughProvider';

const PHASE_ORDER: WalkthroughPhase[] = ['welcome', 'connect', 'automate', 'review', 'done'];

/**
 * Horizontal progress bar showing the walkthrough narrative arc phases.
 * Completed phases get a filled checkmark; the current phase is highlighted;
 * future phases are dimmed.
 */
const WalkthroughProgressBar = () => {
  const { state, phaseLabels, phaseIcons } = useWalkthroughUI();
  const { t } = useT();

  const currentIdx = PHASE_ORDER.indexOf(state.phase);

  return (
    <div
      className="flex items-center gap-1 mb-6"
      role="progressbar"
      aria-valuenow={currentIdx + 1}
      aria-valuemin={1}
      aria-valuemax={PHASE_ORDER.length}>
      {PHASE_ORDER.map((phase, idx) => {
        const isCompleted = idx < currentIdx;
        const isCurrent = idx === currentIdx;
        const isFuture = idx > currentIdx;

        return (
          <div key={phase} className="flex items-center gap-1 flex-1 last:flex-none">
            {/* Phase dot */}
            <div
              className={`
                flex items-center justify-center w-8 h-8 rounded-full text-sm shrink-0
                transition-all duration-300
                ${isCompleted ? 'bg-[#2F6EF4] text-white' : ''}
                ${isCurrent ? 'bg-[#2F6EF4] text-white ring-2 ring-[#2F6EF4]/30' : ''}
                ${isFuture ? 'bg-stone-200 dark:bg-neutral-700 text-stone-400 dark:text-neutral-500' : ''}
              `}
              aria-label={`${phaseLabels[phase]}${
                isCompleted
                  ? ` (${t('walkthrough.progress.completed', 'completed')})`
                  : isCurrent
                    ? ` (${t('walkthrough.progress.current', 'current')})`
                    : ''
              }`}>
              {isCompleted ? '✓' : phaseIcons[phase]}
            </div>

            {/* Connector line */}
            {idx < PHASE_ORDER.length - 1 && (
              <div
                className={`
                  flex-1 h-0.5 rounded-full transition-colors duration-300
                  ${isCompleted ? 'bg-[#2F6EF4]' : 'bg-stone-200 dark:bg-neutral-700'}
                `}
              />
            )}
          </div>
        );
      })}
    </div>
  );
};

export default WalkthroughProgressBar;
