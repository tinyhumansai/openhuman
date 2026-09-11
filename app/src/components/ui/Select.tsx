/**
 * Radix `Select` — the SECOND select in this layer, alongside `NativeSelect`.
 *
 * THE SPLIT IS DELIBERATE. Do not "unify" these into one component; the two
 * are not interchangeable and collapsing them regresses one caller set or the
 * other.
 *
 * - **`NativeSelect` stays the default.** Use it for long lists — model
 *   pickers, timezone lists, locale lists, anything unbounded or driven by a
 *   remote catalogue. It is a real `<select>`: the OS renders the popup, so a
 *   few hundred options cost nothing, the mobile route tree gets the native
 *   wheel picker, and WebDriver drives it with `selectByVisibleText`.
 *   Radix `Select` virtualizes nothing and renders every option as a DOM node.
 * - **This component is for SHORT lists (roughly <= 20 options) that need
 *   custom item rendering** — an icon, a status dot, a two-line label, a
 *   description. A native `<option>` can hold text and nothing else, which is
 *   the one thing it genuinely cannot do.
 *
 * If a list is short but renders as plain text, `NativeSelect` is still the
 * right answer: this one costs a portal, a focus trap and a popper.
 */
import { Select as SelectPrimitive } from 'radix-ui';
import { type ComponentPropsWithoutRef, forwardRef, type ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { CheckIcon } from './icons';

export const SelectRoot = SelectPrimitive.Root;
export const SelectValue = SelectPrimitive.Value;
export const SelectGroup = SelectPrimitive.Group;

export type SelectSize = 'sm' | 'md';

export interface SelectTriggerProps extends ComponentPropsWithoutRef<
  typeof SelectPrimitive.Trigger
> {
  inputSize?: SelectSize;
  children: ReactNode;
}

export const SelectTrigger = forwardRef<HTMLButtonElement, SelectTriggerProps>(
  ({ inputSize = 'md', className, children, ...rest }, ref) => (
    <SelectPrimitive.Trigger
      ref={ref}
      data-slot="select-trigger"
      data-size={inputSize}
      className={cn(
        'inline-flex w-full items-center justify-between gap-2 rounded-lg border border-line-strong bg-surface text-sm text-content',
        'transition-colors duration-150',
        'focus:border-primary-500 focus:outline-hidden focus:ring-2 focus:ring-primary-500/20',
        'data-placeholder:text-content-muted',
        'data-disabled:cursor-not-allowed data-disabled:opacity-50',
        inputSize === 'sm' ? 'h-8 px-2.5' : 'h-9 px-3',
        className
      )}
      {...rest}>
      {children}
      <SelectPrimitive.Icon asChild>
        <svg aria-hidden="true" viewBox="0 0 12 12" className="h-3 w-3 shrink-0 text-content-muted">
          <path
            d="M2 4l4 4 4-4"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  )
);
SelectTrigger.displayName = 'SelectTrigger';

export interface SelectContentProps extends ComponentPropsWithoutRef<
  typeof SelectPrimitive.Content
> {
  /** Portal target — see `DialogContent`. Defaults to `document.body`. */
  container?: HTMLElement | null;
}

/**
 * `position="popper"` rather than the Radix default `item-aligned`:
 * item-aligned positioning measures the selected item and the viewport to
 * overlay the trigger, which needs real layout. The app renders inside
 * transparent panels and constrained webviews where that overlay reads wrong,
 * and jsdom has no layout at all.
 */
export const SelectContent = forwardRef<HTMLDivElement, SelectContentProps>(
  ({ className, position = 'popper', sideOffset = 6, container, children, ...rest }, ref) => (
    <SelectPrimitive.Portal container={container ?? undefined}>
      <SelectPrimitive.Content
        ref={ref}
        data-slot="select-content"
        position={position}
        sideOffset={position === 'popper' ? sideOffset : undefined}
        className={cn(
          'z-50 overflow-hidden rounded-xl border border-line bg-surface p-1 shadow-large',
          'animate-fade-in focus:outline-hidden',
          position === 'popper' && 'w-(--radix-select-trigger-width)',
          className
        )}
        {...rest}>
        <SelectPrimitive.Viewport data-slot="select-viewport" className="max-h-72 overflow-y-auto">
          {children}
        </SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  )
);
SelectContent.displayName = 'SelectContent';

export const SelectItem = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof SelectPrimitive.Item>
>(({ className, children, ...rest }, ref) => (
  <SelectPrimitive.Item
    ref={ref}
    data-slot="select-item"
    className={cn(
      'relative flex cursor-pointer select-none items-center gap-2 rounded-lg py-1.5 pl-2.5 pr-7 text-sm text-content outline-hidden',
      'data-highlighted:bg-surface-hover',
      'data-disabled:pointer-events-none data-disabled:opacity-50',
      className
    )}
    {...rest}>
    <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    <SelectPrimitive.ItemIndicator className="absolute right-2 inline-flex items-center">
      <CheckIcon className="h-3.5 w-3.5 text-primary-500" />
    </SelectPrimitive.ItemIndicator>
  </SelectPrimitive.Item>
));
SelectItem.displayName = 'SelectItem';

export const SelectLabel = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(({ className, ...rest }, ref) => (
  <SelectPrimitive.Label
    ref={ref}
    data-slot="select-label"
    className={cn('px-2.5 py-1.5 text-micro font-medium uppercase text-content-muted', className)}
    {...rest}
  />
));
SelectLabel.displayName = 'SelectLabel';

export const SelectSeparator = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<typeof SelectPrimitive.Separator>
>(({ className, ...rest }, ref) => (
  <SelectPrimitive.Separator
    ref={ref}
    data-slot="select-separator"
    className={cn('my-1 h-px bg-line-subtle', className)}
    {...rest}
  />
));
SelectSeparator.displayName = 'SelectSeparator';

export default SelectRoot;
