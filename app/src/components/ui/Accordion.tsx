import { cva, type VariantProps } from 'class-variance-authority';
import { Accordion as AccordionPrimitive } from 'radix-ui';
import { type ComponentPropsWithRef, type ReactNode } from 'react';

import { cn } from '../../lib/cn';

/**
 * Radix `Accordion` — the multi-section disclosure primitive.
 *
 * What the primitive buys over the hand-rolled `useState(open)` disclosures
 * scattered through the app: roving focus across headers (arrow keys, Home/End),
 * correct `aria-expanded`/`aria-controls` wiring, `type="single" | "multiple"`
 * exclusivity handled once, and `data-state` on every part so the open styling
 * is driven by the primitive instead of by re-deriving `open` in a class string.
 *
 * ANIMATION. Only the tailwind.config.js keyframes are used here — no
 * `tailwindcss-animate`. Radix mounts `Content` only while open (there is no
 * `forceMount` on these wrappers), so the open transition is the one that runs:
 * `data-[state=open]:animate-fade-up`. Radix additionally publishes
 * `--radix-accordion-content-height` (and `--radix-accordion-content-width`) on
 * the content element, measured before paint. A caller that wants a true
 * height collapse can `forceMount` and pass
 * `className="h-(--radix-accordion-content-height) transition-[height]"`;
 * we do not do that by default because a fixed height clips the padding of the
 * section during the frame the browser is still measuring it.
 */
export const accordionVariants = cva('w-full', {
  variants: {
    variant: {
      /** Rows separated by a hairline, no container chrome. */
      plain: 'divide-y divide-line',
      /** Each item is its own bordered card with breathing room between. */
      card: 'flex flex-col gap-2',
      /** One bordered container, hairlines between the rows inside it. */
      contained: 'divide-y divide-line rounded-xl border border-line bg-surface',
    },
  },
  defaultVariants: { variant: 'plain' },
});

export const accordionItemVariants = cva('', {
  variants: {
    variant: {
      plain: '',
      card: 'overflow-hidden rounded-xl border border-line bg-surface',
      contained: 'first:rounded-t-xl last:rounded-b-xl',
    },
  },
  defaultVariants: { variant: 'plain' },
});

export const accordionTriggerVariants = cva(
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

export const accordionContentVariants = cva(
  // `overflow-hidden` keeps the fade-up translate from painting outside the row.
  'overflow-hidden text-content-secondary data-[state=open]:animate-fade-up',
  {
    variants: {
      size: { sm: 'px-3 pb-2 text-xs', md: 'px-4 pb-3 text-sm', lg: 'px-5 pb-4 text-sm' },
    },
    defaultVariants: { size: 'md' },
  }
);

export type AccordionVariant = NonNullable<VariantProps<typeof accordionVariants>['variant']>;
export type AccordionSize = NonNullable<VariantProps<typeof accordionTriggerVariants>['size']>;

export type AccordionRootProps = ComponentPropsWithRef<typeof AccordionPrimitive.Root> &
  VariantProps<typeof accordionVariants>;

export const AccordionRoot = ({ className, variant, ...rest }: AccordionRootProps) => (
  <AccordionPrimitive.Root
    data-slot="accordion"
    data-variant={variant ?? 'plain'}
    className={cn(accordionVariants({ variant }), className)}
    // Radix types `Root` as a discriminated union on `type` ("single" |
    // "multiple"). Destructuring `className`/`variant` off that union widens
    // the remainder to the union of both members, which TS will no longer
    // narrow back to a single arm — so the spread needs the cast. The
    // discrimination is still enforced at the call site, where the caller
    // passes `type` against `AccordionRootProps`.
    {...(rest as ComponentPropsWithRef<typeof AccordionPrimitive.Root>)}
  />
);

export interface AccordionItemProps
  extends
    ComponentPropsWithRef<typeof AccordionPrimitive.Item>,
    VariantProps<typeof accordionItemVariants> {}

export const AccordionItem = ({ className, variant, ...rest }: AccordionItemProps) => (
  <AccordionPrimitive.Item
    data-slot="accordion-item"
    data-variant={variant ?? 'plain'}
    className={cn(accordionItemVariants({ variant }), className)}
    {...rest}
  />
);

export interface AccordionTriggerProps
  extends
    ComponentPropsWithRef<typeof AccordionPrimitive.Trigger>,
    VariantProps<typeof accordionTriggerVariants> {
  /** The disclosure chevron. Opt out when the caller renders its own affordance. */
  showChevron?: boolean;
  /** Class names for the `<h3>` Radix wraps every trigger in. */
  headerClassName?: string;
  children?: ReactNode;
}

/**
 * Radix wraps every trigger in a `Header` element, which is what makes the
 * accordion navigable by heading in a screen reader. Callers should not have to
 * remember that, so the wrapper is part of this component rather than a
 * separate export they can forget.
 */
export const AccordionTrigger = ({
  className,
  headerClassName,
  size,
  showChevron = true,
  children,
  ...rest
}: AccordionTriggerProps) => (
  <AccordionPrimitive.Header data-slot="accordion-header" className={cn('flex', headerClassName)}>
    <AccordionPrimitive.Trigger
      data-slot="accordion-trigger"
      data-size={size ?? 'md'}
      className={cn(accordionTriggerVariants({ size }), className)}
      {...rest}>
      {children}
      {showChevron ? (
        <svg
          data-slot="accordion-chevron"
          aria-hidden="true"
          focusable="false"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className={cn(
            'h-4 w-4 shrink-0 text-content-muted transition-transform duration-200',
            'motion-reduce:transition-none group-data-[state=open]:rotate-180'
          )}>
          <path d="m6 9 6 6 6-6" />
        </svg>
      ) : null}
    </AccordionPrimitive.Trigger>
  </AccordionPrimitive.Header>
);

export interface AccordionContentProps
  extends
    ComponentPropsWithRef<typeof AccordionPrimitive.Content>,
    VariantProps<typeof accordionContentVariants> {}

export const AccordionContent = ({ className, size, ...rest }: AccordionContentProps) => (
  <AccordionPrimitive.Content
    data-slot="accordion-content"
    data-size={size ?? 'md'}
    className={cn(accordionContentVariants({ size }), className)}
    {...rest}
  />
);

export default AccordionRoot;
