import { Popover as PopoverPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

export const PopoverRoot = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;
export const PopoverClose = PopoverPrimitive.Close;

export interface PopoverContentProps extends ComponentPropsWithoutRef<
  typeof PopoverPrimitive.Content
> {
  container?: HTMLElement | null;
}

/**
 * Floating content anchored to a trigger. Replaces the nine files that
 * hand-rolled outside-click dismissal with their own `mousedown` listeners —
 * none of which handled focus, escape, or collision with the viewport edge.
 */
export const PopoverContent = ({
  className,
  align = 'center',
  sideOffset = 6,
  container,
  ...rest
}: PopoverContentProps) => (
  <PopoverPrimitive.Portal container={container ?? undefined}>
    <PopoverPrimitive.Content
      data-slot="popover-content"
      align={align}
      sideOffset={sideOffset}
      className={cn(
        'z-50 rounded-xl border border-line bg-surface p-3 text-sm text-content shadow-large',
        'animate-fade-in focus:outline-hidden',
        className
      )}
      {...rest}
    />
  </PopoverPrimitive.Portal>
);

export default PopoverRoot;
