import { type ReactNode } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';
import { ModalShell } from './ModalShell';

export interface ConfirmDialogProps {
  title: ReactNode;
  body: ReactNode;
  titleId?: string;
  confirmLabel?: ReactNode;
  cancelLabel?: ReactNode;
  busy?: boolean;
  busyLabel?: ReactNode;
  confirmDisabled?: boolean;
  destructive?: boolean;
  /**
   * `data-testid` for the dialog panel. No default — see `ModalShell.testId`.
   */
  testId?: string;
  /**
   * `data-testid` for the confirming button.
   *
   * Defaults to the value this component has always hardcoded, so every
   * existing call site and the specs reading it are unaffected.
   */
  confirmTestId?: string;
  /**
   * `data-testid` for the cancelling button.
   *
   * Deliberately undefaulted, unlike `confirmTestId`: there is no historical
   * value to preserve, and emitting an invented one would add an attribute to
   * every dialog in the app for the benefit of the handful migrating here.
   */
  cancelTestId?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * The three labels default through `useT()` rather than to English literals.
 *
 * They used to be hardcoded English defaults in the signature, which slipped
 * past `i18n:react:check` — that audit walks JSX for `aria-label`/`placeholder`
 * /`title`/`alt`/`label`, and a default parameter value is none of those. So
 * every confirm dialog that did not pass explicit labels rendered "Confirm" /
 * "Cancel" / "Working…" in English regardless of the user's locale.
 */
export function ConfirmDialog({
  title,
  body,
  titleId = 'confirm-dialog-title',
  confirmLabel,
  cancelLabel,
  busy = false,
  busyLabel,
  confirmDisabled = false,
  destructive = false,
  testId,
  confirmTestId = 'confirm-dialog-confirm',
  cancelTestId,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useT();
  const resolvedConfirmLabel = confirmLabel ?? t('common.confirm');
  const resolvedCancelLabel = cancelLabel ?? t('common.cancel');
  const resolvedBusyLabel = busyLabel ?? t('common.working');

  return (
    <ModalShell
      title={title}
      titleId={titleId}
      testId={testId}
      onClose={onCancel}
      maxWidthClassName="max-w-sm"
      closePolicy={busy ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            size="sm"
            data-testid={cancelTestId}
            onClick={onCancel}
            disabled={busy}>
            {resolvedCancelLabel}
          </Button>
          <Button
            variant="primary"
            size="sm"
            tone={destructive ? 'danger' : undefined}
            data-testid={confirmTestId}
            onClick={onConfirm}
            disabled={busy || confirmDisabled}>
            {busy ? resolvedBusyLabel : resolvedConfirmLabel}
          </Button>
        </div>
      }>
      <div className="text-sm text-content-secondary">{body}</div>
    </ModalShell>
  );
}
