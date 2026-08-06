import { type ReactNode } from 'react';

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
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  body,
  titleId = 'confirm-dialog-title',
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  busy = false,
  busyLabel = 'Working…',
  confirmDisabled = false,
  destructive = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  return (
    <ModalShell
      title={title}
      titleId={titleId}
      onClose={onCancel}
      maxWidthClassName="max-w-sm"
      closePolicy={busy ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button
            variant="primary"
            size="sm"
            tone={destructive ? 'danger' : undefined}
            data-testid="confirm-dialog-confirm"
            onClick={onConfirm}
            disabled={busy || confirmDisabled}>
            {busy ? busyLabel : confirmLabel}
          </Button>
        </div>
      }>
      <div className="text-sm text-content-secondary">{body}</div>
    </ModalShell>
  );
}
