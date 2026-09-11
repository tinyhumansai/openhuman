import { Avatar as AvatarPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, forwardRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * Radix `Avatar` — an image with a fallback that only appears once the image
 * has actually failed or is still loading.
 *
 * That delay is the whole point. The hand-rolled avatars in the app render
 * `<img onError={() => setBroken(true)}>` next to initials, which flashes the
 * initials for a frame on every render before the cached image paints. Radix
 * tracks the load state and swaps once, and `delayMs` on the fallback
 * suppresses the flash entirely on a fast connection.
 *
 * **No `size` prop, deliberately.** Nothing else in `src/components/ui/` that
 * is purely a box (Card, Separator, ListRow) invents a size scale, and an
 * avatar is used at half a dozen sizes across chat, settings and the mascot
 * surfaces. Size it from the caller: `<AvatarRoot className="h-8 w-8">`. The
 * root supplies shape, overflow and the fallback's centring; the caller
 * supplies the box.
 */
export const AvatarRoot = forwardRef<
  HTMLSpanElement,
  ComponentPropsWithoutRef<typeof AvatarPrimitive.Root>
>(({ className, ...rest }, ref) => (
  <AvatarPrimitive.Root
    ref={ref}
    data-slot="avatar"
    className={cn(
      'relative inline-flex h-9 w-9 shrink-0 overflow-hidden rounded-full bg-surface-strong',
      className
    )}
    {...rest}
  />
));
AvatarRoot.displayName = 'AvatarRoot';

export const AvatarImage = forwardRef<
  HTMLImageElement,
  ComponentPropsWithoutRef<typeof AvatarPrimitive.Image>
>(({ className, ...rest }, ref) => (
  <AvatarPrimitive.Image
    ref={ref}
    data-slot="avatar-image"
    className={cn('h-full w-full object-cover', className)}
    {...rest}
  />
));
AvatarImage.displayName = 'AvatarImage';

export const AvatarFallback = forwardRef<
  HTMLSpanElement,
  ComponentPropsWithoutRef<typeof AvatarPrimitive.Fallback>
>(({ className, ...rest }, ref) => (
  <AvatarPrimitive.Fallback
    ref={ref}
    data-slot="avatar-fallback"
    className={cn(
      'flex h-full w-full items-center justify-center bg-surface-strong text-xs font-medium uppercase text-content-secondary',
      className
    )}
    {...rest}
  />
));
AvatarFallback.displayName = 'AvatarFallback';

export default AvatarRoot;
