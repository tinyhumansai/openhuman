import { DropdownMenu as DropdownMenuPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

export const DropdownMenuRoot = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;
export const DropdownMenuGroup = DropdownMenuPrimitive.Group;
export const DropdownMenuLabel = DropdownMenuPrimitive.Label;

export interface DropdownMenuContentProps extends ComponentPropsWithoutRef<
  typeof DropdownMenuPrimitive.Content
> {
  container?: HTMLElement | null;
}

/** A keyboard-navigable action menu — arrow keys, typeahead, roving focus. */
export const DropdownMenuContent = ({
  className,
  align = 'end',
  sideOffset = 6,
  container,
  ...rest
}: DropdownMenuContentProps) => (
  <DropdownMenuPrimitive.Portal container={container ?? undefined}>
    <DropdownMenuPrimitive.Content
      data-slot="dropdown-menu-content"
      align={align}
      sideOffset={sideOffset}
      className={cn(
        'z-50 min-w-40 overflow-hidden rounded-xl border border-line bg-surface p-1 shadow-large',
        'animate-fade-in focus:outline-hidden',
        className
      )}
      {...rest}
    />
  </DropdownMenuPrimitive.Portal>
);

export const DropdownMenuItem = ({
  className,
  ...rest
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item>) => (
  <DropdownMenuPrimitive.Item
    data-slot="dropdown-menu-item"
    className={cn(
      'flex cursor-pointer select-none items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm text-content outline-hidden',
      'data-highlighted:bg-surface-hover',
      'data-disabled:pointer-events-none data-disabled:opacity-50',
      className
    )}
    {...rest}
  />
);

export const DropdownMenuSeparator = ({
  className,
  ...rest
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator>) => (
  <DropdownMenuPrimitive.Separator
    className={cn('my-1 h-px bg-line-subtle', className)}
    {...rest}
  />
);

export default DropdownMenuRoot;
