import { type ReactNode, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import { useEscapeKey } from '../../../hooks/useEscapeKey';
import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import { CloseIcon } from '../../ui/icons';

interface SettingsModalFrameProps {
  /** Invoked on X click, Esc, or backdrop click. */
  onClose: () => void;
  children: ReactNode;
  /** id of the element labelling the dialog, if any. */
  labelledBy?: string;
}

/**
 * Presentational chrome for the desktop Settings modal: a portalled, dimmed
 * backdrop and a centered, full-app-size card with a floating close button.
 *
 * Purely presentational — it owns no routing/state so it can be unit-tested in
 * isolation. Reuses the same primitives as {@link ModalShell} (Esc handling,
 * focus restore, `createPortal`, `CloseIcon`) but lays the card out as a flex
 * container for the two-column body, with the close affordance floated in the
 * top-right corner instead of a title bar.
 */
export function SettingsModalFrame({ onClose, children, labelledBy }: SettingsModalFrameProps) {
  const { t } = useT();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEscapeKey(onClose);

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    dialogRef.current?.focus();
    return () => previousFocus?.focus?.();
  }, []);

  // Portal into #root (not document.body) so the modal stays inside the app's
  // tested subtree — `#root`-scoped checks (and E2E specs reading
  // `#root.innerText()`) see the routed panel. Falls back to body if absent.
  const portalTarget = document.getElementById('root') ?? document.body;

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      data-testid="settings-modal-backdrop"
      onClick={event => {
        if (event.target === event.currentTarget) onClose();
      }}>
      {/* Positioning wrapper sized to the card. The close button is a sibling of
          the card so it can float just above the top-right corner — outside the
          card surface — and never overlap panel content. No overflow clip here. */}
      <div
        className="relative mx-4 flex h-[80vh] w-full max-w-5xl"
        onClick={event => event.stopPropagation()}>
        <Button
          variant="secondary"
          iconOnly
          size="sm"
          aria-label={t('common.close')}
          data-testid="settings-modal-close"
          onClick={onClose}
          className="absolute bottom-full right-0 mb-2 h-8 w-8 rounded-full text-content-muted shadow-md hover:text-content-secondary">
          <CloseIcon className="h-4 w-4" />
        </Button>
        <div
          ref={dialogRef}
          role="dialog"
          aria-modal="true"
          aria-labelledby={labelledBy}
          aria-label={labelledBy ? undefined : t('nav.settings')}
          tabIndex={-1}
          data-testid="settings-modal-card"
          className="flex h-full w-full overflow-hidden rounded-2xl bg-surface shadow-xl animate-fade-up focus:outline-none">
          {children}
        </div>
      </div>
    </div>,
    portalTarget
  );
}
