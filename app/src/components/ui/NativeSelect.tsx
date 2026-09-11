import { forwardRef, type SelectHTMLAttributes } from 'react';

import { cn } from '../../lib/cn';

export type NativeSelectSize = 'sm' | 'md';

export interface NativeSelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  inputSize?: NativeSelectSize;
  'data-testid'?: string;
}

/**
 * A styled native `<select>`, kept deliberately alongside Radix `Select`.
 *
 * Radix `Select` is worth it for short lists that need custom item rendering.
 * It is the wrong tool for model pickers and timezone lists — it virtualizes
 * nothing, so a few hundred options are slow, and it is the hardest primitive
 * to drive under jsdom. The native control also remains the right answer on the
 * mobile route tree, where the OS picker beats any HTML popup.
 */
/**
 * KNOWN LIMITATION: the `%23a3a3a3` stroke is a literal, so the chevron does not
 * follow the theme. A data-URI cannot read a CSS variable, and the fix that does
 * work -- a wrapper element with an inline `currentColor` SVG -- moves where the
 * caller's width class has to land, which would touch every `NativeSelect` call
 * site. Left deliberately, not overlooked.
 */
const CHEVRON_BG =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath d='M2 4l4 4 4-4' stroke='%23a3a3a3' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round' fill='none'/%3E%3C/svg%3E\")";

const NativeSelect = forwardRef<HTMLSelectElement, NativeSelectProps>(
  ({ inputSize = 'md', className, 'data-testid': testId, style, ...rest }, ref) => (
    <select
      ref={ref}
      data-slot="native-select"
      data-size={inputSize}
      data-testid={testId}
      className={cn(
        'block cursor-pointer appearance-none rounded-lg border border-line-strong bg-surface bg-no-repeat pr-7 text-sm text-content',
        // `@plugin '@tailwindcss/forms'` puts `padding: 0.5rem 0.75rem` on bare
        // `select`. Only the horizontal half was ever overridden here, so 16px
        // of vertical padding stayed inside the 32px `h-8` box: 8 + 8 padding +
        // 2 border leaves 14px for a 20px line box, and the overflow clipped
        // every descender. `py-0` + an explicit `leading-none` centres the text
        // in the box the height already declares.
        'py-0 leading-none',
        'transition-colors duration-150',
        'focus:border-primary-500 focus:outline-hidden focus:ring-2 focus:ring-primary-500/20',
        'disabled:cursor-not-allowed disabled:opacity-50',
        inputSize === 'sm' ? 'h-8 pl-2.5' : 'h-9 pl-3',
        className
      )}
      style={{
        backgroundImage: CHEVRON_BG,
        backgroundPosition: 'right 0.5rem center',
        backgroundSize: '12px 12px',
        ...style,
      }}
      {...rest}
    />
  )
);
NativeSelect.displayName = 'NativeSelect';

export default NativeSelect;
