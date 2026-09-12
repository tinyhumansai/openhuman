import { cva, type VariantProps } from 'class-variance-authority';
import { type AriaRole, type ComponentPropsWithRef, createContext, useContext } from 'react';

import { cn } from '../../lib/cn';

/**
 * A static status surface — no Radix primitive, because there is no
 * interaction to manage: an alert is read, not operated.
 *
 * ROLE. `role="alert"` is applied to `destructive` and `warning` only. It maps
 * to an assertive live region, which interrupts whatever a screen reader is
 * saying; using it for an `info` panel that is simply present on page load
 * makes every visit talk over itself. Informational variants stay a plain
 * container and are read in document order.
 *
 * A caller always wins over that default by passing `role` itself, and the
 * presence of the prop — not its value — is what decides. Three cases:
 *
 * - `role="status"` — the notice arrives after load but is not urgent (a
 *   polite live region). `McpServerPanel.tsx`'s "could not open the config
 *   file" line is exactly this, and pairs it with `aria-live="polite"`.
 * - `role={undefined}` — the notice is a *load-present* warning or error: it
 *   was on the page before the reader arrived, so announcing it assertively
 *   talks over the page it is describing. Six of the notices this primitive
 *   replaces are of that shape. This is the documented opt-out, and it works
 *   because the check is `'role' in props`, not `props.role != null`.
 * - anything else — used verbatim.
 *
 * DENSITY. `default` is the standalone-notice geometry (`rounded-xl px-4 py-3
 * text-sm`). `compact` (`rounded-lg px-3 py-2 text-xs`) is the inline geometry
 * used by notices that sit inside a panel or a form, which is what nearly
 * every hand-rolled notice in the app already is — a bare swap at the default
 * density would visibly enlarge them. `compact` also carries down to
 * `AlertDescription`, so the body text shrinks with the box rather than
 * staying `text-sm` inside a `text-xs` container.
 */
export const alertVariants = cva('relative flex w-full gap-3 rounded-xl border px-4 py-3 text-sm', {
  variants: {
    variant: {
      default: 'border-line bg-surface text-content',
      info: 'border-primary-200 bg-primary-50 text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-200',
      success:
        'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-200',
      warning:
        'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200',
      destructive:
        'border-coral-200 bg-coral-50 text-coral-600 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-200',
    },
    // The default density contributes NOTHING: the geometry stays in the base
    // string above so an un-densified caller's class attribute is unchanged
    // to the byte. `compact` overrides it, and `cn()`'s tailwind-merge drops
    // the four base utilities it conflicts with.
    density: { default: '', compact: 'rounded-lg px-3 py-2 text-xs' },
  },
  defaultVariants: { variant: 'default', density: 'default' },
});

export type AlertVariant = NonNullable<VariantProps<typeof alertVariants>['variant']>;
export type AlertDensity = NonNullable<VariantProps<typeof alertVariants>['density']>;

/** The variants urgent enough to justify an assertive live region. */
const ASSERTIVE_VARIANTS: readonly AlertVariant[] = ['destructive', 'warning'];

/**
 * Lets `AlertDescription` size itself against the box it is in without every
 * call site repeating the density. Outside an `Alert` it reads `default`, so
 * a standalone `AlertDescription` is unchanged.
 */
const AlertDensityContext = createContext<AlertDensity>('default');

export interface AlertProps
  extends ComponentPropsWithRef<'div'>, VariantProps<typeof alertVariants> {}

export const Alert = (props: AlertProps) => {
  const { className, variant, density, ...rest } = props;
  const resolved: AlertVariant = variant ?? 'default';
  const resolvedDensity: AlertDensity = density ?? 'default';
  // Presence, not value — `role={undefined}` is a deliberate opt-out of the
  // tone-derived default and must not fall through to it.
  const role: AriaRole | undefined =
    'role' in props ? props.role : ASSERTIVE_VARIANTS.includes(resolved) ? 'alert' : undefined;
  return (
    <AlertDensityContext.Provider value={resolvedDensity}>
      <div
        data-slot="alert"
        data-variant={resolved}
        role={role}
        className={cn(alertVariants({ variant, density }), className)}
        {...rest}
      />
    </AlertDensityContext.Provider>
  );
};

export const AlertTitle = ({ className, ...rest }: ComponentPropsWithRef<'div'>) => (
  <div
    data-slot="alert-title"
    className={cn('font-medium leading-snug tracking-tight', className)}
    {...rest}
  />
);

export const AlertDescription = ({ className, ...rest }: ComponentPropsWithRef<'div'>) => {
  const density = useContext(AlertDensityContext);
  return (
    <div
      data-slot="alert-description"
      // The size leads, because tailwind-merge treats a font size as
      // conflicting with `leading-*` (Tailwind's `text-sm/6` shorthand sets
      // both). Appending `text-xs` would silently strip the line height.
      className={cn(
        density === 'compact' ? 'text-xs' : 'text-sm',
        'leading-relaxed opacity-90',
        className
      )}
      {...rest}
    />
  );
};

export default Alert;
