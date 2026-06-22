import { useT } from '../../lib/i18n/I18nContext';
import WalkthroughActionCard from './WalkthroughActionCard';
import WalkthroughProgressBar from './WalkthroughProgressBar';
import { useWalkthroughUI } from './WalkthroughProvider';

/**
 * Renders the current walkthrough phase: progress bar, phase header with icon
 * and description, and the list of action cards for the current phase.
 *
 * When the phase is 'review', shows a summary of what was configured.
 * When the phase is 'done', shows a celebration screen.
 */
const WalkthroughPhasePanel = () => {
  const { state, advance, skip, phaseLabels, phaseIcons } = useWalkthroughUI();
  const { t } = useT();

  const { phase, steps } = state;

  // ── Done screen ──────────────────────────────────────────────────────
  if (phase === 'done') {
    return (
      <div className="text-center py-8 animate-in fade-in slide-in-from-bottom-4 duration-500">
        <div className="text-5xl mb-4">{phaseIcons.done}</div>
        <h2 className="text-xl font-bold text-stone-900 dark:text-neutral-100 mb-2">
          {t('walkthrough.done.title', "You're all set!")}
        </h2>
        <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-sm mx-auto">
          {t(
            'walkthrough.done.description',
            'Your assistant is ready to help. Connections are set up and automations are configured.'
          )}
        </p>
      </div>
    );
  }

  // ── Review screen ────────────────────────────────────────────────────
  if (phase === 'review') {
    return (
      <div className="animate-in fade-in slide-in-from-bottom-4 duration-500">
        <WalkthroughProgressBar />

        <div className="text-center mb-6">
          <span className="text-3xl block mb-2">{phaseIcons.review}</span>
          <h2 className="text-lg font-bold text-stone-900 dark:text-neutral-100">
            {phaseLabels.review}
          </h2>
          <p className="text-sm text-stone-500 dark:text-neutral-400 mt-1">
            {t(
              'walkthrough.review.description',
              "Here's a summary of what you've set up. You can always change these later."
            )}
          </p>
        </div>

        {/* Summary cards — show what was completed */}
        <div className="space-y-2 mb-6">
          {steps.length === 0 ? (
            <p className="text-sm text-stone-400 dark:text-neutral-500 text-center py-4">
              {state.skipped
                ? t(
                    'walkthrough.review.skipped',
                    'You skipped the setup. You can configure these anytime in Settings.'
                  )
                : t('walkthrough.review.empty', 'No actions completed yet.')}
            </p>
          ) : (
            steps.map(step => <WalkthroughActionCard key={step.key} step={step} />)
          )}
        </div>

        <div className="text-center">
          <button
            type="button"
            onClick={advance}
            className="rounded-xl bg-[#2F6EF4] px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-[#2559c7]">
            {t('walkthrough.review.finish', 'Finish setup')}
          </button>
        </div>
      </div>
    );
  }

  // ── Active phase (welcome, connect, automate) ────────────────────────
  return (
    <div className="animate-in fade-in slide-in-from-bottom-4 duration-500">
      <WalkthroughProgressBar />

      {/* Phase header */}
      <div className="text-center mb-6">
        <span className="text-3xl block mb-2">{phaseIcons[phase]}</span>
        <h2 className="text-lg font-bold text-stone-900 dark:text-neutral-100">
          {phaseLabels[phase]}
        </h2>
        {phase === 'connect' && (
          <p className="text-sm text-stone-500 dark:text-neutral-400 mt-1">
            {t(
              'walkthrough.connect.description',
              'Connect the tools you already use. Each connection gives your assistant new abilities.'
            )}
          </p>
        )}
        {phase === 'automate' && (
          <p className="text-sm text-stone-500 dark:text-neutral-400 mt-1">
            {t(
              'walkthrough.automate.description',
              'Choose what your assistant should handle automatically.'
            )}
          </p>
        )}
        {phase === 'welcome' && (
          <p className="text-sm text-stone-500 dark:text-neutral-400 mt-1">
            {t(
              'walkthrough.welcome.description',
              'Let us guide you through setting up your AI assistant.'
            )}
          </p>
        )}
      </div>

      {/* Action cards */}
      <div className="space-y-2 mb-6">
        {steps.map(step => (
          <WalkthroughActionCard key={step.key} step={step} />
        ))}
      </div>

      {/* Skip button */}
      <div className="text-center">
        <button
          type="button"
          onClick={skip}
          className="text-xs text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 transition-colors underline underline-offset-2">
          {t('walkthrough.phase.skip', 'Skip this step')}
        </button>
      </div>
    </div>
  );
};

export default WalkthroughPhasePanel;
