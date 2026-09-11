import { cva, type VariantProps } from 'class-variance-authority';
import { RadioGroup as RadioGroupPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, type ElementRef, forwardRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * Radix `RadioGroup` replacing the hand-rolled radio lists across the app —
 * lists that rendered `<button>`s or bare `<input type="radio">`s without a
 * group role, so a screen reader announced "1 of 1" for every option and arrow
 * keys did nothing.
 *
 * What the primitive gives us for free: roving focus (arrow keys move AND
 * select, Home/End jump), a real `role="radiogroup"`, a hidden native input for
 * form participation, and `data-state` so the indicator dot is driven by the
 * primitive rather than by re-deriving `checked` in a class string.
 *
 * The focus ring offsets against `surface`, never a hardcoded white — an offset
 * that ignores the user's theme reads wrong on any tinted surface.
 */
export const radioGroupItemVariants = cva(
  'relative inline-flex shrink-0 items-center justify-center rounded-full border bg-surface ' +
    'border-line-strong transition-colors duration-150 ' +
    'hover:border-primary-500 ' +
    'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500 ' +
    'focus-visible:ring-offset-2 focus-visible:ring-offset-surface ' +
    'data-[state=checked]:border-primary-500 ' +
    'disabled:cursor-not-allowed disabled:opacity-50',
  {
    variants: { size: { sm: 'h-3.5 w-3.5', md: 'h-4 w-4', lg: 'h-5 w-5' } },
    defaultVariants: { size: 'md' },
  }
);

const indicatorSize = { sm: 'h-1.5 w-1.5', md: 'h-2 w-2', lg: 'h-2.5 w-2.5' } as const;

export type RadioGroupRootProps = ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Root>;

export const RadioGroupRoot = forwardRef<
  ElementRef<typeof RadioGroupPrimitive.Root>,
  RadioGroupRootProps
>(({ className, ...rest }, ref) => (
  <RadioGroupPrimitive.Root
    ref={ref}
    data-slot="radio-group"
    className={cn('flex flex-col gap-2', className)}
    {...rest}
  />
));
RadioGroupRoot.displayName = 'RadioGroupRoot';

export interface RadioGroupItemProps
  extends
    ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Item>,
    VariantProps<typeof radioGroupItemVariants> {}

export const RadioGroupItem = forwardRef<
  ElementRef<typeof RadioGroupPrimitive.Item>,
  RadioGroupItemProps
>(({ className, size = 'md', ...rest }, ref) => (
  <RadioGroupPrimitive.Item
    ref={ref}
    data-slot="radio-group-item"
    data-size={size}
    className={cn(radioGroupItemVariants({ size }), className)}
    {...rest}>
    <RadioGroupPrimitive.Indicator
      data-slot="radio-group-indicator"
      className={cn('block rounded-full bg-primary-500', indicatorSize[size ?? 'md'])}
    />
  </RadioGroupPrimitive.Item>
));
RadioGroupItem.displayName = 'RadioGroupItem';

export default RadioGroupRoot;
