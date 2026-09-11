import { cva, type VariantProps } from 'class-variance-authority';
import { Toggle as TogglePrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, type ElementRef, forwardRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * A two-state button. Radix owns `aria-pressed` and the `data-state=on|off`
 * flag; everything visual is borrowed from `Button` so a toggle can sit in a
 * toolbar beside a real button without reading as a foreign control — the same
 * `xs/sm/md/lg/xl` height-and-radius scale, the same padding compounds, the
 * same `focus-visible` ring offset against `surface`.
 *
 * Only two of Button's three variants make sense here: a toggle's *pressed*
 * state is what carries emphasis, so an always-primary base would leave the on
 * and off states indistinguishable.
 *
 * TONE IS APPENDED, NOT SUBSTITUTED — same trap as `Button`. cva's
 * `compoundVariants` concatenate, so a danger toggle's class list holds both
 * the default and the coral utility and only `cn()`'s tailwind-merge pass picks
 * the winner, last-wins. Never render `toggleVariants(...)` outside `cn()`.
 */
export const toggleVariants = cva(
  'inline-flex items-center justify-center gap-2 font-medium transition-colors duration-150 ' +
    'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-offset-2 ' +
    'focus-visible:ring-offset-surface focus-visible:ring-primary-500/25 ' +
    'disabled:opacity-40 disabled:pointer-events-none',
  {
    variants: {
      variant: {
        secondary:
          'bg-surface text-content-secondary border border-line-strong hover:bg-surface-hover ' +
          'data-[state=on]:bg-surface-strong data-[state=on]:text-content ' +
          'data-[state=on]:border-line-strong',
        tertiary:
          'bg-transparent text-content-muted hover:bg-surface-hover hover:text-content-secondary ' +
          'data-[state=on]:bg-surface-strong data-[state=on]:text-content',
      },
      tone: { default: '', danger: '' },
      size: {
        xs: 'h-6 text-xs rounded-sm',
        sm: 'h-[30px] text-sm rounded-md',
        md: 'h-9 text-sm rounded-lg',
        lg: 'h-11 text-base rounded-lg',
        xl: 'h-14 text-base rounded-xl',
      },
      iconOnly: { true: '', false: '' },
    },
    compoundVariants: [
      {
        tone: 'danger',
        class:
          'text-coral-600 hover:bg-coral-50 focus-visible:ring-coral-500/25 ' +
          'data-[state=on]:bg-coral-500 data-[state=on]:text-content-inverted ' +
          'dark:text-coral-400 dark:hover:bg-coral-500/10',
      },
      { iconOnly: false, size: 'xs', class: 'px-2' },
      { iconOnly: false, size: 'sm', class: 'px-3' },
      { iconOnly: false, size: 'md', class: 'px-4' },
      { iconOnly: false, size: 'lg', class: 'px-5' },
      { iconOnly: false, size: 'xl', class: 'px-7' },
      { iconOnly: true, size: 'xs', class: 'w-6' },
      { iconOnly: true, size: 'sm', class: 'w-[30px]' },
      { iconOnly: true, size: 'md', class: 'w-9' },
      { iconOnly: true, size: 'lg', class: 'w-11' },
      { iconOnly: true, size: 'xl', class: 'w-14' },
    ],
    defaultVariants: { variant: 'tertiary', tone: 'default', size: 'md', iconOnly: false },
  }
);

export interface ToggleProps
  extends
    ComponentPropsWithoutRef<typeof TogglePrimitive.Root>,
    VariantProps<typeof toggleVariants> {
  /** Stable, content-free identifier consumed by the app-wide analytics tracker. */
  analyticsId?: string;
}

const Toggle = forwardRef<ElementRef<typeof TogglePrimitive.Root>, ToggleProps>(
  (
    {
      variant = 'tertiary',
      tone = 'default',
      size = 'md',
      iconOnly = false,
      analyticsId,
      className,
      ...rest
    },
    ref
  ) => (
    <TogglePrimitive.Root
      ref={ref}
      data-slot="toggle"
      data-variant={variant}
      data-tone={tone}
      data-size={size}
      data-analytics-id={analyticsId}
      className={cn(toggleVariants({ variant, tone, size, iconOnly }), className)}
      {...rest}
    />
  )
);
Toggle.displayName = 'Toggle';

export default Toggle;
