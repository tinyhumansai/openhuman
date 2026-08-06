/**
 * TransferHandleModal — confirm + execute a Tiny Place handle transfer (GH-4929).
 *
 * A handle transfer is DESTRUCTIVE and irreversible for the sender: on success
 * the recipient becomes the handle's sole owner. So this modal states that
 * plainly, requires an explicit recipient, requires the user to re-type the
 * handle to confirm intent, and takes an explicit confirm click — and it fails
 * **closed**: on any error it keeps the dialog open with the message and never
 * reports success. The core handler resolves the recipient @handle and
 * read-back-confirms the new owner before this promise resolves, so a resolved
 * transfer means the reassignment actually landed.
 */
import debugFactory from 'debug';
import { useCallback, useState } from 'react';

import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import { useT } from '../../lib/i18n/I18nContext';
import { apiClient } from '../AgentWorldShell';

// Namespaced already ('agentworld:identity'), so messages carry no prefix.
const debug = debugFactory('agentworld:identity');

interface TransferHandleModalProps {
  /** The handle being transferred away (without a leading @). */
  handle: string;
  onClose: () => void;
  /** Called after a confirmed, read-back-verified transfer. */
  onTransferred: () => void;
}

/** Normalize a handle for comparison: strip leading @, trim, lowercase. */
function normalizeHandle(value: string): string {
  return value.trim().replace(/^@+/, '').toLowerCase();
}

export default function TransferHandleModal({
  handle,
  onClose,
  onTransferred,
}: TransferHandleModalProps) {
  const { t } = useT();
  const [recipient, setRecipient] = useState('');
  const [confirmText, setConfirmText] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleClean = handle.replace(/^@+/, '');
  // Guard the irreversible action: the user must re-type the exact handle.
  const confirmMatches = normalizeHandle(confirmText) === normalizeHandle(handleClean);

  const submit = useCallback(async () => {
    const target = recipient.trim().replace(/^@+/, '');
    if (!target) {
      setError(t('agentWorld.transferHandle.recipientRequired'));
      return;
    }
    // Belt-and-suspenders: the button is disabled without a match, but never
    // execute a destructive transfer unless the typed confirmation matches.
    if (!confirmMatches) {
      setError(t('agentWorld.transferHandle.confirmMismatch'));
      return;
    }
    setSubmitting(true);
    setError(null);
    // Never log the handle or recipient — both identify a user.
    debug('handle transfer requested');
    try {
      // Send the normalized handle (not the raw prop) so the invariant is local
      // and doesn't rest on every caller pre-cleaning the value (#4998 review).
      await apiClient.registry.transfer(handleClean, target);
      debug('handle transfer confirmed');
      onTransferred();
      onClose();
    } catch (err) {
      // Fail closed: keep the dialog open, show why, report no success.
      // Log only the status (no raw error — it can carry backend/SDK detail);
      // the raw message still surfaces in the UI via setError.
      debug('handle transfer failed');
      setError(String(err));
      setSubmitting(false);
    }
  }, [recipient, confirmMatches, handleClean, t, onTransferred, onClose]);

  return (
    <ModalShell
      title={t('agentWorld.transferHandle.title')}
      titleId="agentworld-transfer-handle-title"
      maxWidthClassName="max-w-sm"
      onClose={submitting ? () => undefined : onClose}>
      <div className="space-y-4" data-testid="transfer-handle-modal">
        <p className="text-sm text-content">@{handleClean}</p>
        <p className="text-xs text-red-600 dark:text-red-400">
          {t('agentWorld.transferHandle.warning')}
        </p>

        <input
          type="text"
          value={recipient}
          onChange={e => {
            setRecipient(e.target.value);
            setError(null);
          }}
          disabled={submitting}
          placeholder={t('agentWorld.transferHandle.recipientPlaceholder')}
          aria-label={t('agentWorld.transferHandle.recipientPlaceholder')}
          className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder-content-faint outline-none focus:border-primary-500"
        />

        {/* Type-to-confirm guard for the irreversible action. */}
        <div className="space-y-1">
          <p className="text-xs text-content-muted">
            {t('agentWorld.transferHandle.confirmLabel')}
          </p>
          <input
            type="text"
            value={confirmText}
            onChange={e => {
              setConfirmText(e.target.value);
              setError(null);
            }}
            disabled={submitting}
            placeholder={`@${handleClean}`}
            aria-label={t('agentWorld.transferHandle.confirmLabel')}
            data-testid="transfer-handle-confirm-input"
            className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder-content-faint outline-none focus:border-primary-500"
          />
        </div>

        {error && (
          <p className="text-xs text-red-600 dark:text-red-400" data-testid="transfer-handle-error">
            {error}
          </p>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onClose} disabled={submitting}>
            {t('common.cancel')}
          </Button>
          <Button
            variant="primary"
            size="sm"
            tone="danger"
            onClick={() => void submit()}
            disabled={submitting || !recipient.trim() || !confirmMatches}
            data-testid="transfer-handle-confirm">
            {submitting
              ? t('agentWorld.transferHandle.submitting')
              : t('agentWorld.transferHandle.confirm')}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
