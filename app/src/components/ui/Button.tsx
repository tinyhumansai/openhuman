import { cva, type VariantProps } from 'class-variance-authority';
import { Slot } from 'radix-ui';
import { type ButtonHTMLAttributes, forwardRef, type ReactNode } from 'react';

import { cn } from '../../lib/cn';

/**
 * The one button in the app. Three hierarchy variants plus an orthogonal tone:
 *
 * - **variant** — visual weight / importance:
 *   - `primary`   the main call-to-action on a surface (Save, Continue, Create)
 *   - `secondary` an alternative of similar weight (Cancel, Back, Import)
 *   - `tertiary`  low-emphasis / text-style action (Skip, links, inline actions)
 * - **tone** — semantic intent layered on any variant:
 *   - `default`   the normal palette
 *   - `danger`    destructive actions (Delete, Remove, Logout) — coral
 *
 * Use `iconOnly` for icon-only affordances (close / refresh / add); it squares
 * the padding — always pass an `aria-label` in that case.
 *
 * ---
 *
 * DANGER TONE IS APPENDED, NOT SUBSTITUTED — and that difference is load-bearing.
 * The previous implementation looked its classes up as `VARIANTS[variant][tone]`,
 * so the danger string *replaced* the default one. cva's `compoundVariants`
 * instead concatenate, so a danger button's class list contains both
 * `bg-primary-500` and `bg-coral-500`; only `cn()`'s tailwind-merge pass
 * resolves that to coral, last-wins.
 *
 * The consequence: never render `buttonVariants(...)` without passing it
 * through `cn()`, or every destructive button in the app silently renders in
 * the primary colour. `Button.test.tsx` asserts the losing class is absent for
 * each tone rather than merely asserting the winning one is present, because
 * only the former catches that failure.
 */
export const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 font-medium transition-colors duration-150 ' +
    'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-offset-2 ' +
    'focus-visible:ring-offset-surface ' +
    'disabled:opacity-40 disabled:pointer-events-none',
  {
    variants: {
      variant: {
        primary:
          'bg-primary-500 text-content-inverted hover:bg-primary-600 active:bg-primary-700 focus-visible:ring-primary-500/25 ' +
          'dark:hover:bg-primary-400 dark:active:bg-primary-600',
        secondary:
          'bg-surface text-content border border-line-strong hover:bg-surface-hover focus-visible:ring-primary-500/25',
        tertiary:
          'bg-transparent text-content-secondary hover:bg-surface-hover focus-visible:ring-primary-500/25',
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
        variant: 'primary',
        tone: 'danger',
        class:
          'bg-coral-500 text-content-inverted hover:bg-coral-600 active:bg-coral-700 focus-visible:ring-coral-500/25 ' +
          'dark:hover:bg-coral-400 dark:active:bg-coral-600',
      },
      {
        variant: 'secondary',
        tone: 'danger',
        class:
          'bg-transparent text-coral-600 border border-coral-300/50 hover:bg-coral-50 focus-visible:ring-coral-500/25 ' +
          'dark:text-coral-400 dark:border-coral-500/40 dark:hover:bg-coral-500/10',
      },
      {
        variant: 'tertiary',
        tone: 'danger',
        class:
          'bg-transparent text-coral-600 hover:bg-coral-50 focus-visible:ring-coral-500/25 ' +
          'dark:text-coral-400 dark:hover:bg-coral-500/10',
      },
      // Horizontal padding for text buttons; square footprints for icon-only.
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
    defaultVariants: { variant: 'primary', tone: 'default', size: 'md', iconOnly: false },
  }
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  /** Render the child element instead of a `<button>` — for links and Radix triggers. */
  asChild?: boolean;
  leadingIcon?: ReactNode;
  trailingIcon?: ReactNode;
  /** Stable, content-free identifier consumed by the app-wide analytics tracker. */
  analyticsId?: string;
}

const Button = forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
  const {
    variant = 'primary',
    tone = 'default',
    size = 'md',
    iconOnly = false,
    asChild = false,
    leadingIcon,
    trailingIcon,
    analyticsId,
    className,
    type,
    children,
    ...rest
  } = props;

  const Comp = asChild ? Slot.Root : 'button';

  return (
    <Comp
      ref={ref}
      // `asChild` delegates to whatever the child renders — forcing `type` onto
      // an `<a>` would emit an invalid attribute.
      type={asChild ? undefined : (type ?? 'button')}
      data-slot="button"
      data-variant={variant}
      data-tone={tone}
      data-size={size}
      className={cn(buttonVariants({ variant, tone, size, iconOnly }), className)}
      data-analytics-id={analyticsId}
      {...rest}>
      {/* `Slot` requires exactly one element child, so under `asChild` the
          caller owns its own layout and the icon slots are not applied. */}
      {asChild ? (
        children
      ) : (
        <>
          {leadingIcon}
          {children}
          {trailingIcon}
        </>
      )}
    </Comp>
  );
});
Button.displayName = 'Button';

export default Button;
