import type { TooltipRenderProps } from 'react-joyride';

/** Emoji accents per step — adds visual personality to each tooltip.
 *  10 entries map to: home-card, home-cta, chat, integrations, channels,
 *  intelligence, settings, quick-access tabs, notifications, final. */
const STEP_ICONS = ['🏠', '💬', '🗨️', '🧩', '📱', '🧠', '⚙️', '⚡', '🔔', '🎉'];

/**
 * Premium tooltip for the post-onboarding Joyride walkthrough.
 *
 * Design: frosted-glass card with smooth entrance animation, step-specific
 * emoji accent, pill progress bar, and polished button styling that matches
 * the OpenHuman design system (ocean primary #2F6EF4, warm neutrals).
 */
const WalkthroughTooltip = ({
  continuous,
  index,
  step,
  backProps,
  primaryProps,
  skipProps,
  tooltipProps,
  size,
  isLastStep,
}: TooltipRenderProps) => {
  const progress = ((index + 1) / size) * 100;
  const icon = STEP_ICONS[index] ?? '✨';

  return (
    <div
      {...tooltipProps}
      className="w-80 font-sans animate-in fade-in slide-in-from-bottom-2 duration-300"
      style={{ animation: 'tooltipEnter 0.3s ease-out' }}>
      {/* Frosted card */}
      <div className="bg-white/95 backdrop-blur-md rounded-2xl shadow-xl border border-stone-200/60 overflow-hidden">
        {/* Progress bar — thin, smooth fill */}
        <div className="h-1 bg-stone-100">
          <div
            className="h-full bg-gradient-to-r from-[#2F6EF4] to-[#5B9BF3] transition-all duration-500 ease-out rounded-r-full"
            style={{ width: `${progress}%` }}
          />
        </div>

        <div className="p-5">
          {/* Header: emoji + title + step counter */}
          <div className="flex items-start gap-3 mb-3">
            <span className="text-2xl shrink-0 mt-0.5" role="img" aria-hidden="true">
              {icon}
            </span>
            <div className="flex-1 min-w-0">
              {step.title && (
                <h3 className="text-[15px] font-semibold text-stone-900 leading-snug">
                  {step.title}
                </h3>
              )}
              <span className="text-[11px] text-stone-400 tabular-nums">
                {index + 1} of {size}
              </span>
            </div>
          </div>

          {/* Body */}
          <div className="text-[13px] text-stone-600 leading-relaxed mb-5">{step.content}</div>

          {/* Actions */}
          <div className="flex items-center gap-2">
            {/* Skip tour */}
            {!isLastStep && (
              <button
                {...skipProps}
                className="text-[11px] text-stone-400 hover:text-stone-600 transition-colors px-2 py-1.5 rounded-lg hover:bg-stone-100">
                Skip tour
              </button>
            )}

            <div className="flex-1" />

            {/* Back */}
            {index > 0 && (
              <button
                {...backProps}
                className="text-[12px] text-stone-500 hover:text-stone-800 border border-stone-200 hover:border-stone-300 transition-all px-4 py-2 rounded-xl hover:shadow-sm">
                Back
              </button>
            )}

            {/* Next / Let's go! */}
            {continuous && (
              <button
                {...primaryProps}
                className="text-[12px] text-white bg-[#2F6EF4] hover:bg-[#2563d4] active:scale-[0.97] transition-all px-4 py-2 rounded-xl font-medium shadow-sm hover:shadow-md">
                {isLastStep ? "Let's go!" : 'Next →'}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default WalkthroughTooltip;
