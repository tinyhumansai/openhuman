import { Dialog as DialogPrimitive } from 'radix-ui';
import { type ReactNode, useEffect } from 'react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';
import { DialogContent, DialogRoot } from './Dialog';
import { CloseIcon } from './icons';

export interface ModalClosePolicy {
  escape?: boolean;
  backdrop?: boolean;
  button?: boolean;
}

export interface ModalShellProps {
  children: ReactNode;
  onClose: () => void;
  title: ReactNode;
  titleId: string;
  subtitle?: ReactNode;
  icon?: ReactNode;
  footer?: ReactNode;
  maxWidthClassName?: string;
  contentClassName?: string;
  panelClassName?: string;
  labelledBy?: string;
  describedBy?: string;
  closePolicy?: ModalClosePolicy;
  /** Portal target — see `DialogContent`. Defaults to `document.body`. */
  container?: HTMLElement | null;
  /**
   * `data-testid` for the dialog panel — the element carrying `role="dialog"`.
   *
   * There is deliberately no default. A hand-rolled overlay migrating onto this
   * shell brings its own hook (`add-account-modal`, `flow-action-confirm`, …),
   * and inventing a default would stamp a new attribute onto all ~40 existing
   * call sites for the benefit of none of them.
   */
  testId?: string;
}

/**
 * The app's modal frame, now backed by Radix `Dialog`.
 *
 * The prop signature is unchanged, so all existing call sites render
 * identically. What Radix adds over the previous hand-rolled portal: a real
 * focus trap (there was none — Tab escaped the dialog into the page behind it),
 * scroll locking, `aria-hidden` on the rest of the tree, and focus restore that
 * survives the focused element being unmounted.
 *
 * `closePolicy` maps onto Radix's escape/outside events by `preventDefault()`,
 * not by dropping handlers: `{ escape: false, backdrop: false, button: false }`
 * — which `ConfirmDialog` uses while a confirm is in flight — must stay
 * genuinely undismissable.
 *
 * FOCUS RESTORE IS STILL OURS, DELIBERATELY. Radix restores focus on the
 * open -> false transition via `onCloseAutoFocus`, but every caller of this
 * component closes by *unmounting* it rather than by flipping `open`. Radix
 * never sees that transition, so relying on it would drop focus to `<body>`
 * on every dialog close — verified against a bare `Dialog.Root`, not assumed.
 * The effect below is the same save/restore the hand-rolled shell used.
 */
export function ModalShell({
  children,
  onClose,
  title,
  titleId,
  subtitle,
  icon,
  footer,
  maxWidthClassName = 'max-w-md',
  contentClassName = 'px-5 py-4',
  panelClassName,
  labelledBy,
  describedBy,
  closePolicy,
  container,
  testId,
}: ModalShellProps) {
  const { t } = useT();
  const allowEscapeClose = closePolicy?.escape ?? true;
  const allowBackdropClose = closePolicy?.backdrop ?? true;
  const allowButtonClose = closePolicy?.button ?? true;

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    return () => previousFocus?.focus?.();
  }, []);

  return (
    <DialogRoot
      open
      onOpenChange={next => {
        if (!next) onClose();
      }}>
      <DialogContent
        container={container}
        data-testid={testId}
        aria-labelledby={labelledBy ?? titleId}
        aria-describedby={describedBy}
        className={cn('mx-4', maxWidthClassName, panelClassName)}
        onEscapeKeyDown={event => {
          if (!allowEscapeClose) event.preventDefault();
        }}
        onPointerDownOutside={event => {
          if (!allowBackdropClose) event.preventDefault();
        }}
        onInteractOutside={event => {
          if (!allowBackdropClose) event.preventDefault();
        }}>
        <div className="flex items-center justify-between border-b border-line-subtle px-5 py-4">
          <div className="flex min-w-0 items-center gap-3">
            {icon ? (
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary-50 text-primary-600">
                {icon}
              </div>
            ) : null}
            <div className="min-w-0">
              {/* Radix requires a Title for a11y. `asChild` keeps the historical
                  h2 + id so `getByRole('heading')` and aria-labelledby still
                  resolve exactly as before. */}
              <DialogPrimitive.Title asChild>
                <h2 id={titleId} className="text-sm font-semibold text-content">
                  {title}
                </h2>
              </DialogPrimitive.Title>
              {subtitle ? <p className="text-xs text-content-muted">{subtitle}</p> : null}
            </div>
          </div>
          {allowButtonClose ? (
            <Button
              iconOnly
              variant="tertiary"
              size="sm"
              aria-label={t('common.close')}
              onClick={onClose}>
              <CloseIcon className="h-4 w-4" />
            </Button>
          ) : null}
        </div>
        <div className={contentClassName}>{children}</div>
        {footer ? <div className="border-t border-line-subtle px-5 py-4">{footer}</div> : null}
      </DialogContent>
    </DialogRoot>
  );
}

export default ModalShell;
