/**
 * ConfirmDialog — in-app confirmation modal for Agent World destructive actions.
 *
 * Replaces native `window.confirm()` (which renders the OS/browser dialog
 * titled "tauri.localhost says …", exposing the internal hostname to users —
 * #4197) with a styled modal consistent with the app's design system, built on
 * the shared [`ModalShell`].
 *
 * The parent owns the action: this component only renders the confirmation and
 * reports the user's decision via `onConfirm` / `onCancel`. Render it
 * conditionally from parent state and await the user's choice before firing the
 * destructive RPC.
 */
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';

export interface ConfirmDialogProps {
  /** Modal header (e.g. "Delete post"). */
  title: string;
  /** Body copy explaining the consequence (e.g. "Delete this post? This can't be undone."). */
  message: string;
  /** Confirm-button label. Defaults to "Delete". */
  confirmLabel?: string;
  /** Cancel-button label. Defaults to "Cancel". */
  cancelLabel?: string;
  /** Render the confirm button with the danger tone (default true — these are destructive). */
  destructive?: boolean;
  /** When true, the confirm button is disabled and shows `busyLabel`. */
  busy?: boolean;
  /** Label shown on the confirm button while `busy` (e.g. "Deleting…"). */
  busyLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function ConfirmDialog({
  title,
  message,
  confirmLabel = 'Delete',
  cancelLabel = 'Cancel',
  destructive = true,
  busy = false,
  busyLabel = 'Deleting…',
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  return (
    <ModalShell
      title={title}
      titleId="agentworld-confirm-title"
      onClose={busy ? () => undefined : onCancel}
      maxWidthClassName="max-w-sm">
      <div className="space-y-4">
        <p className="text-sm text-content-secondary" data-testid="confirm-dialog-message">
          {message}
        </p>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button
            variant="primary"
            size="sm"
            tone={destructive ? 'danger' : 'default'}
            onClick={onConfirm}
            disabled={busy}
            data-testid="confirm-dialog-confirm">
            {busy ? busyLabel : confirmLabel}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
