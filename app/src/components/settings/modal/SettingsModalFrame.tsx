import { type ReactNode, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import { useEscapeKey } from '../../../hooks/useEscapeKey';
import { useT } from '../../../lib/i18n/I18nContext';
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
        <button
          type="button"
          aria-label={t('common.close')}
          data-testid="settings-modal-close"
          onClick={onClose}
          className="absolute bottom-full right-0 mb-2 flex h-8 w-8 items-center justify-center rounded-full border border-stone-200 bg-white text-stone-500 shadow-md transition-colors hover:bg-stone-100 hover:text-stone-700 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-200">
          <CloseIcon className="h-4 w-4" />
        </button>
        <div
          ref={dialogRef}
          role="dialog"
          aria-modal="true"
          aria-labelledby={labelledBy}
          aria-label={labelledBy ? undefined : t('nav.settings')}
          tabIndex={-1}
          data-testid="settings-modal-card"
          className="flex h-full w-full overflow-hidden rounded-2xl bg-white shadow-xl animate-fade-up focus:outline-none dark:bg-neutral-900">
          {children}
        </div>
      </div>
    </div>,
    portalTarget
  );
}
