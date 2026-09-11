import { VisuallyHidden as VisuallyHiddenPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, type ElementRef, forwardRef } from 'react';

import { cn } from '../../lib/cn';

export interface VisuallyHiddenProps extends ComponentPropsWithoutRef<
  typeof VisuallyHiddenPrimitive.Root
> {}

/**
 * Content that is removed from the visual layout but stays in the accessibility
 * tree.
 *
 * Required, not decorative: Radix's `Dialog`/`AlertDialog` warn (and screen
 * readers announce nothing useful) when a dialog has no `Title`. An icon-only
 * dialog therefore wraps its title in this rather than dropping it — the
 * alternative, `display: none`, would hide it from assistive technology too.
 *
 * The clipping is Radix's own inline style; the class slot exists only so a
 * caller can layer positioning on it.
 */
export const VisuallyHidden = forwardRef<
  ElementRef<typeof VisuallyHiddenPrimitive.Root>,
  VisuallyHiddenProps
>(({ className, ...rest }, ref) => (
  <VisuallyHiddenPrimitive.Root
    ref={ref}
    data-slot="visually-hidden"
    className={cn(className)}
    {...rest}
  />
));
VisuallyHidden.displayName = 'VisuallyHidden';

export default VisuallyHidden;
