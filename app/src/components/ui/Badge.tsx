import { cva, type VariantProps } from 'class-variance-authority';
import { type HTMLAttributes, type ReactNode } from 'react';

import { cn } from '../../lib/cn';

export const badgeVariants = cva(
  'inline-flex items-center rounded-md border px-1.5 py-0.5 text-[11px] font-medium leading-none',
  {
    variants: {
      variant: {
        neutral: 'border-line bg-surface-subtle text-content-secondary dark:border-line-strong',
        primary:
          'border-primary-200 bg-primary-50 text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300',
        success:
          'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300',
        warning:
          'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
        danger:
          'border-coral-200 bg-coral-50 text-coral-600 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300',
      },
    },
    defaultVariants: { variant: 'neutral' },
  }
);

export type BadgeVariant = NonNullable<VariantProps<typeof badgeVariants>['variant']>;

export interface BadgeProps
  extends Omit<HTMLAttributes<HTMLSpanElement>, 'children'>, VariantProps<typeof badgeVariants> {
  children: ReactNode;
  className?: string;
  'data-testid'?: string;
}

/**
 * Pill label. Any extra span attribute (`title`, `aria-*`, `onClick`, …) is
 * forwarded to the rendered element, so a hand-rolled pill carrying a tooltip
 * or an event handler can adopt this primitive without losing it. The
 * component-owned attributes are applied after `...rest` so the default render
 * is identical whether or not extra props are passed.
 */
const Badge = ({ variant, children, className, 'data-testid': testId, ...rest }: BadgeProps) => (
  <span
    {...rest}
    data-slot="badge"
    data-variant={variant ?? 'neutral'}
    data-testid={testId}
    className={cn(badgeVariants({ variant }), className)}>
    {children}
  </span>
);

export default Badge;
