import { cva, type VariantProps } from 'class-variance-authority';
import { Collapsible as CollapsiblePrimitive } from 'radix-ui';
import { type ComponentPropsWithRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * Radix `Collapsible` — the single-section case of the disclosure family.
 *
 * Reach for this over `Accordion` when there is exactly one region and no
 * exclusivity to coordinate: it is the same `data-state` contract and the same
 * `aria-expanded`/`aria-controls` wiring without the roving-focus header list.
 *
 * ANIMATION. Same rule as `Accordion`: only the tailwind.config.js keyframes,
 * no `tailwindcss-animate`. Content mounts on open, so
 * `data-[state=open]:animate-fade-in` is the transition that runs. Radix
 * publishes `--radix-collapsible-content-height` (and
 * `--radix-collapsible-content-width`) on the content element for callers that
 * `forceMount` and want a measured height collapse
 * (`h-(--radix-collapsible-content-height) transition-[height]`); the
 * default does not set a height so the region can reflow freely.
 */
export const collapsibleVariants = cva('', {
  variants: {
    variant: {
      /** No container chrome — the caller owns the surrounding layout. */
      plain: '',
      /** A self-contained bordered card. */
      card: 'overflow-hidden rounded-xl border border-line bg-surface',
    },
  },
  defaultVariants: { variant: 'plain' },
});

export const collapsibleTriggerVariants = cva(
  [
    'group flex w-full items-center justify-between gap-2 text-left font-medium',
    'text-content transition-colors hover:bg-surface-hover',
    'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25',
    'disabled:cursor-not-allowed disabled:opacity-50',
  ],
  {
    variants: {
      size: { sm: 'px-3 py-2 text-xs', md: 'px-4 py-3 text-sm', lg: 'px-5 py-4 text-base' },
    },
    defaultVariants: { size: 'md' },
  }
);

export const collapsibleContentVariants = cva(
  'overflow-hidden text-content-secondary data-[state=open]:animate-fade-in',
  {
    variants: {
      size: { sm: 'px-3 pb-2 text-xs', md: 'px-4 pb-3 text-sm', lg: 'px-5 pb-4 text-sm' },
    },
    defaultVariants: { size: 'md' },
  }
);

export type CollapsibleVariant = NonNullable<VariantProps<typeof collapsibleVariants>['variant']>;
export type CollapsibleSize = NonNullable<VariantProps<typeof collapsibleTriggerVariants>['size']>;

export interface CollapsibleRootProps
  extends
    ComponentPropsWithRef<typeof CollapsiblePrimitive.Root>,
    VariantProps<typeof collapsibleVariants> {}

export const CollapsibleRoot = ({ className, variant, ...rest }: CollapsibleRootProps) => (
  <CollapsiblePrimitive.Root
    data-slot="collapsible"
    data-variant={variant ?? 'plain'}
    className={cn(collapsibleVariants({ variant }), className)}
    {...rest}
  />
);

export interface CollapsibleTriggerProps
  extends
    ComponentPropsWithRef<typeof CollapsiblePrimitive.Trigger>,
    VariantProps<typeof collapsibleTriggerVariants> {}

export const CollapsibleTrigger = ({ className, size, ...rest }: CollapsibleTriggerProps) => (
  <CollapsiblePrimitive.Trigger
    data-slot="collapsible-trigger"
    data-size={size ?? 'md'}
    className={cn(collapsibleTriggerVariants({ size }), className)}
    {...rest}
  />
);

export interface CollapsibleContentProps
  extends
    ComponentPropsWithRef<typeof CollapsiblePrimitive.Content>,
    VariantProps<typeof collapsibleContentVariants> {}

export const CollapsibleContent = ({ className, size, ...rest }: CollapsibleContentProps) => (
  <CollapsiblePrimitive.Content
    data-slot="collapsible-content"
    data-size={size ?? 'md'}
    className={cn(collapsibleContentVariants({ size }), className)}
    {...rest}
  />
);

export default CollapsibleRoot;
