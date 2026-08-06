import { type ReactNode, useEffect } from 'react';
import { createPortal } from 'react-dom';

import { useDismissLayer } from '../../hooks/useDismissLayer';
import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';
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
}

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
}: ModalShellProps) {
  const { t } = useT();
  const allowEscapeClose = closePolicy?.escape ?? true;
  const allowBackdropClose = closePolicy?.backdrop ?? true;
  const allowButtonClose = closePolicy?.button ?? true;
  const { layerRef, onPointerDownCapture } = useDismissLayer({
    onDismiss: onClose,
    dismissOnEscape: allowEscapeClose,
    dismissOnOutsidePointer: allowBackdropClose,
  });

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    layerRef.current?.focus();
    return () => previousFocus?.focus?.();
  }, [layerRef]);

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onPointerDownCapture={onPointerDownCapture}>
      <div
        ref={element => {
          layerRef.current = element;
        }}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? titleId}
        aria-describedby={describedBy}
        tabIndex={-1}
        className={`w-full ${maxWidthClassName} mx-4 rounded-2xl bg-surface shadow-xl overflow-hidden animate-fade-up focus:outline-none ${panelClassName ?? ''}`}
        onClick={event => event.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-line-subtle px-5 py-4">
          <div className="flex min-w-0 items-center gap-3">
            {icon ? (
              <div className="w-9 h-9 rounded-xl bg-primary-50 flex items-center justify-center text-primary-600 flex-shrink-0">
                {icon}
              </div>
            ) : null}
            <div className="min-w-0">
              <h2 id={titleId} className="text-sm font-semibold text-content">
                {title}
              </h2>
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
              <CloseIcon className="w-4 h-4" />
            </Button>
          ) : null}
        </div>
        <div className={contentClassName}>{children}</div>
        {footer ? <div className="border-t border-line-subtle px-5 py-4">{footer}</div> : null}
      </div>
    </div>,
    document.body
  );
}
