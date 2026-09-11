import { cva, type VariantProps } from 'class-variance-authority';
import { AlertDialog as AlertDialogPrimitive } from 'radix-ui';
import {
  type ComponentPropsWithoutRef,
  type ElementRef,
  forwardRef,
  type HTMLAttributes,
  type ReactNode,
} from 'react';

import { cn } from '../../lib/cn';

export const AlertDialogRoot = AlertDialogPrimitive.Root;
export const AlertDialogTrigger = AlertDialogPrimitive.Trigger;

/**
 * The destructive-confirm overlay, and the lower-level primitive that
 * `ConfirmDialog` could later be rebuilt on. `ConfirmDialog` keeps its own API
 * and is untouched by this file.
 *
 * WHY THIS IS NOT `Dialog`: an alert dialog is a decision the user must make,
 * so Radix's `AlertDialog` deliberately does **not** dismiss on an outside
 * click — a stray click next to a "Delete everything?" prompt must not count as
 * an answer. It also traps focus onto `Cancel` by default and marks the content
 * `role="alertdialog"`, which is why both `Action` and `Cancel` have to be
 * rendered: a dialog with no cancel path is one whose only escape is Escape.
 *
 * Do not "fix" the missing outside-click dismissal by adding an
 * `onInteractOutside` that closes — that is the whole point of the component.
 */
export interface AlertDialogOverlayProps extends ComponentPropsWithoutRef<
  typeof AlertDialogPrimitive.Overlay
> {}

export const AlertDialogOverlay = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Overlay>,
  AlertDialogOverlayProps
>(({ className, ...rest }, ref) => (
  <AlertDialogPrimitive.Overlay
    ref={ref}
    data-slot="alert-dialog-overlay"
    // `surface-overlay` is the themed scrim token — a literal `bg-black/50`
    // would ignore the user's theme, and raw palette utilities are banned in
    // this directory by `lint:ui-tokens`.
    className={cn('fixed inset-0 z-50 bg-surface-overlay/50 backdrop-blur-sm', className)}
    {...rest}
  />
));
AlertDialogOverlay.displayName = 'AlertDialogOverlay';

export interface AlertDialogContentProps extends ComponentPropsWithoutRef<
  typeof AlertDialogPrimitive.Content
> {
  /**
   * Portal target. Defaults to `document.body`.
   *
   * Same reason `DialogContent` takes one: the mascot, notch and overlay
   * windows are transparent panels rendering their own React root, and a scrim
   * portalled into their `document.body` paints an opaque sheet over a window
   * that is supposed to be see-through. Those roots pass their own container.
   */
  container?: HTMLElement | null;
  overlayClassName?: string;
  children: ReactNode;
}

export const AlertDialogContent = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Content>,
  AlertDialogContentProps
>(({ className, overlayClassName, container, children, ...rest }, ref) => (
  <AlertDialogPrimitive.Portal container={container ?? undefined}>
    <AlertDialogOverlay className={overlayClassName} />
    <AlertDialogPrimitive.Content
      ref={ref}
      data-slot="alert-dialog-content"
      className={cn(
        'fixed left-1/2 top-1/2 z-50 w-full max-w-sm -translate-x-1/2 -translate-y-1/2',
        'overflow-hidden rounded-2xl border border-line bg-surface p-5 shadow-large',
        'animate-dialog-in focus:outline-hidden',
        className
      )}
      {...rest}>
      {children}
    </AlertDialogPrimitive.Content>
  </AlertDialogPrimitive.Portal>
));
AlertDialogContent.displayName = 'AlertDialogContent';

export const AlertDialogTitle = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Title>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Title>
>(({ className, ...rest }, ref) => (
  <AlertDialogPrimitive.Title
    ref={ref}
    data-slot="alert-dialog-title"
    className={cn('text-base font-semibold text-content', className)}
    {...rest}
  />
));
AlertDialogTitle.displayName = 'AlertDialogTitle';

export const AlertDialogDescription = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Description>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Description>
>(({ className, ...rest }, ref) => (
  <AlertDialogPrimitive.Description
    ref={ref}
    data-slot="alert-dialog-description"
    className={cn('mt-2 text-sm text-content-secondary', className)}
    {...rest}
  />
));
AlertDialogDescription.displayName = 'AlertDialogDescription';

export const AlertDialogFooter = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="alert-dialog-footer"
      className={cn('mt-5 flex justify-end gap-2', className)}
      {...rest}
    />
  )
);
AlertDialogFooter.displayName = 'AlertDialogFooter';

/**
 * `tone` is appended, never substituted — the same cva shape `Button` uses, and
 * with the same consequence: the danger class list contains both
 * `bg-primary-500` and `bg-coral-500`, and only `cn()`'s tailwind-merge pass
 * resolves it to coral, last-wins. Never render these variants without `cn()`.
 */
const alertDialogActionVariants = cva(
  'inline-flex items-center justify-center gap-2 h-[30px] px-3 rounded-md text-sm font-medium ' +
    'transition-colors duration-150 focus-visible:outline-hidden focus-visible:ring-2 ' +
    'focus-visible:ring-offset-2 focus-visible:ring-offset-surface ' +
    'disabled:opacity-40 disabled:pointer-events-none',
  {
    variants: {
      tone: {
        default:
          'bg-primary-500 text-content-inverted hover:bg-primary-600 active:bg-primary-700 ' +
          'focus-visible:ring-primary-500/25',
        danger:
          'bg-coral-500 text-content-inverted hover:bg-coral-600 active:bg-coral-700 ' +
          'focus-visible:ring-coral-500/25',
      },
    },
    defaultVariants: { tone: 'danger' },
  }
);

export interface AlertDialogActionProps
  extends
    ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Action>,
    VariantProps<typeof alertDialogActionVariants> {}

/** The confirming choice. Defaults to the danger tone — this overlay exists for
 *  destructive decisions; pass `tone="default"` for a benign one. */
export const AlertDialogAction = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Action>,
  AlertDialogActionProps
>(({ className, tone = 'danger', ...rest }, ref) => (
  <AlertDialogPrimitive.Action
    ref={ref}
    data-slot="alert-dialog-action"
    data-tone={tone}
    className={cn(alertDialogActionVariants({ tone }), className)}
    {...rest}
  />
));
AlertDialogAction.displayName = 'AlertDialogAction';

/** The escape hatch. Radix focuses this on open, so it must always be rendered. */
export const AlertDialogCancel = forwardRef<
  ElementRef<typeof AlertDialogPrimitive.Cancel>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Cancel>
>(({ className, ...rest }, ref) => (
  <AlertDialogPrimitive.Cancel
    ref={ref}
    data-slot="alert-dialog-cancel"
    className={cn(
      'inline-flex items-center justify-center gap-2 h-[30px] px-3 rounded-md text-sm font-medium',
      'border border-line-strong bg-surface text-content hover:bg-surface-hover',
      'transition-colors duration-150 focus-visible:outline-hidden focus-visible:ring-2',
      'focus-visible:ring-primary-500/25 focus-visible:ring-offset-2 focus-visible:ring-offset-surface',
      'disabled:opacity-40 disabled:pointer-events-none',
      className
    )}
    {...rest}
  />
));
AlertDialogCancel.displayName = 'AlertDialogCancel';

export default AlertDialogRoot;
