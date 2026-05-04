import type { TooltipRenderProps } from 'react-joyride';

/**
 * Custom tooltip component for the post-onboarding Joyride walkthrough.
 * Matches the design system: ocean primary #2F6EF4, Inter font, Tailwind classes.
 *
 * Receives all props from react-joyride's `tooltipComponent` API.
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
  return (
    <div
      {...tooltipProps}
      className="bg-white rounded-xl shadow-lg border border-stone-200 p-5 max-w-xs w-72 font-sans">
      {/* Header: title + step counter */}
      <div className="flex items-start justify-between mb-2 gap-2">
        {step.title && (
          <h3 className="text-sm font-semibold text-stone-900 leading-snug flex-1">
            {step.title as string}
          </h3>
        )}
        <span className="shrink-0 text-xs text-stone-400 tabular-nums mt-px">
          {index + 1} of {size}
        </span>
      </div>

      {/* Body */}
      <p className="text-sm text-stone-600 leading-relaxed mb-4">{step.content as string}</p>

      {/* Progress dots */}
      <div className="flex items-center justify-center gap-1.5 mb-4">
        {Array.from({ length: size }).map((_, i) => (
          <span
            key={i}
            className={`block rounded-full transition-all duration-300 ${
              i === index ? 'w-4 h-1.5 bg-[#2F6EF4]' : 'w-1.5 h-1.5 bg-stone-300'
            }`}
          />
        ))}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-2">
        {/* Skip tour — only show while not on the last step */}
        {!isLastStep && (
          <button
            {...skipProps}
            className="text-xs text-stone-400 hover:text-stone-600 transition-colors px-1 py-1 rounded-md">
            Skip tour
          </button>
        )}

        <div className="flex-1" />

        {/* Back — only after first step */}
        {index > 0 && (
          <button
            {...backProps}
            className="text-xs text-stone-500 hover:text-stone-800 border border-stone-300 hover:border-stone-400 transition-colors px-3 py-1.5 rounded-lg">
            Back
          </button>
        )}

        {/* Next / Finish */}
        {continuous && (
          <button
            {...primaryProps}
            className="text-xs text-white bg-[#2F6EF4] hover:bg-[#2563d4] transition-colors px-3 py-1.5 rounded-lg font-medium">
            {isLastStep ? 'Finish' : 'Next'}
          </button>
        )}
      </div>
    </div>
  );
};

export default WalkthroughTooltip;
