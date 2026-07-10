import { type ReactNode, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';
import { CloseIcon } from './icons';

interface ModalShellProps {
  children: ReactNode;
  onClose: () => void;
  title: ReactNode;
  titleId: string;
  subtitle?: ReactNode;
  icon?: ReactNode;
  maxWidthClassName?: string;
  contentClassName?: string;
  labelledBy?: string;
}

export function ModalShell({
  children,
  onClose,
  title,
  titleId,
  subtitle,
  icon,
  maxWidthClassName = 'max-w-md',
  contentClassName = 'px-5 py-4',
  labelledBy,
}: ModalShellProps) {
  const { t } = useT();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEscapeKey(onClose);

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    return () => previousFocus?.focus?.();
  }, []);

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onClick={event => {
        if (event.target === event.currentTarget) onClose();
      }}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy ?? titleId}
        tabIndex={-1}
        className={`w-full ${maxWidthClassName} mx-4 rounded-2xl bg-surface shadow-xl overflow-hidden animate-fade-up focus:outline-none`}
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
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('common.close')}
            onClick={onClose}>
            <CloseIcon className="w-4 h-4" />
          </Button>
        </div>
        <div className={contentClassName}>{children}</div>
      </div>
    </div>,
    document.body
  );
}
